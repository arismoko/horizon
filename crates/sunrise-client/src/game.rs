//! The playable game loop: streaming world, player simulation at 20 ticks/s,
//! block interaction, inventory and HUD.
#[cfg(test)]
use key_codes::drop_held_behind_dialog;
use key_codes::{
    follow_named_keymap, gameplay_has_keyboard, glfw_button, glfw_modifiers, keyboard_button,
};
use loading_screen::{
    App, Loading, leave_fullscreen_after, preflight_resources, run_event_loop,
    unclean_start_recovery,
};
use motion_ids::{motion_at, motion_block_entity};
#[cfg(test)]
use resource_load::load_resources;
use resource_load::{
    Progress, StartupLoadContext, launch_world, load_resources_for_mods_with_limit,
    load_resources_sync, named_scope, synchronous_startup,
};
/// The content tables the world plays with, as field-level borrows: the
/// world's data packs' over the game's own. The `Game::items`/`recipes`/
/// `stations` methods read the same tables, but taking `&self` blocks a
/// `&mut self.…` in the same scope; these expand to a match on the fields,
/// which borrows stay disjoint from.
macro_rules! world_items {
    ($s:ident) => {
        match (&$s.server_items, &$s.world_tables) {
            (Some(items), _) => &**items,
            (None, Some(t)) => &t.items,
            (None, None) => $s.res.items(),
        }
    };
}
macro_rules! world_variants {
    ($s:ident) => {
        match &$s.world_tables {
            Some(t) => &*t.variants,
            None => $s.res.variants(),
        }
    };
}
macro_rules! world_recipes {
    ($s:ident) => {
        match &$s.world_tables {
            Some(t) => &t.recipes,
            None => $s.res.recipes(),
        }
    };
}
/// The painting variants the world places with (`Painting.create`): its own
/// registry and `#minecraft:placeable`, else the prepared root's.
macro_rules! world_paintings {
    ($s:ident) => {
        match &$s.world_tables {
            Some(t) => &t.painting_variants,
            None => &$s.painting_variants,
        }
    };
}
macro_rules! world_providers {
    ($s:ident) => {
        match &$s.world_tables {
            Some(t) => &t.providers,
            None => $s.res.providers(),
        }
    };
}
macro_rules! world_stations {
    ($s:ident) => {
        match &$s.world_tables {
            Some(t) => &t.stations,
            None => $s.res.stations(),
        }
    };
}
/// The biome table the world plays with (`Game::biomes`), as a field-level
/// borrow: the world's packs' over the game's own.
macro_rules! world_biomes {
    ($s:expr) => {
        match &$s.world_biomes {
            Some(b) => b,
            None => $s.res.biomes(),
        }
    };
}
/// The acting player's stacks as field-level borrows, in vanilla `EquipmentSlot` order.
macro_rules! acting_equipment {
    ($s:expr) => {{
        use sunrise_items::equipment::{BODY, SADDLE};
        [
            $s.inventory[$s.selected].as_ref(),
            $s.offhand.as_ref(),
            $s.armor[3].as_ref(),
            $s.armor[2].as_ref(),
            $s.armor[1].as_ref(),
            $s.armor[0].as_ref(),
            $s.equipment.get(BODY),
            $s.equipment.get(SADDLE),
        ]
    }};
}
/// This side's level data for a tick as a field-level borrow, so it stands beside the tick's own
/// borrows of the game. Build it before taking a field mutably.
macro_rules! game_side {
    ($s:expr) => {
        $crate::game::native_surface::GameSide {
            dim: $s.dim,
            level: $s.native_lifecycle.epoch().and_then(|epoch| {
                let key = $s.dim.key();
                let id = mod_api::__private::dimension_id(key)?;
                Some(mod_api::LevelRef::with_generation(
                    epoch,
                    key,
                    $s.levels.runtime.generation_of(id),
                ))
            }),
            facts: $s.res.facts(),
            tables: $crate::game::native_block_tables::BlockTables::new(&$s.res, &$s.pack),
            biomes: world_biomes!($s),
            behaviours: $s.res.behaviours(),
            features: $s
                .remote
                .as_ref()
                .map_or($s.level.enabled_features.as_arc(), |remote| {
                    &remote.features
                }),
            items: $s.res.items(),
            damage_types: &$s.health_resources.damage,
            player_equipment: acting_equipment!($s),
            hit_borrows: $crate::game::native_surface::HitBorrows {
                living_types: $s.res.living_type_keys(),
                attributes: &$s.motion.attributes,
                player_effects: &$s.potions.effects,
                player_uuid: $s.lan_acting_uuid,
                player_attachments: &$s.player_state.attachments,
                entity_attachments: &*$s.entity_attachments,
            },
            tag_items: if $s.options.execution_role.is_client() {
                $s.remote.as_ref().and_then(|remote| remote.tags.as_ref())
            } else {
                $s.session_lock.is_some().then(|| $s.res.items())
            },
        }
    };
}
/// The world's packs' content files, as a field-level borrow (see the macros).
fn pack_files(world_tables: &Option<WorldTables>) -> sunrise_world::loot::PackOverlay<'_> {
    match world_tables {
        Some(t) => sunrise_world::loot::PackOverlay {
            conditions: &t.conditions,
            files: &t.files,
            priority_layers: &t.priority_layers,
        },
        None => sunrise_world::loot::PackOverlay::NONE,
    }
}
// The modules of the game/<area>/ folders, by name: an area folder is not a module.
#[path = "game/runtime/activity.rs"]
mod activity;
#[path = "game/interface/advancements/mod.rs"]
mod advancements;
#[path = "game/interface/advancements_screen.rs"]
mod advancements_screen;
#[path = "game/runtime/agent/mod.rs"]
mod agent;
#[path = "game/runtime/agent_settings.rs"]
mod agent_settings;
#[path = "game/entities/ahead.rs"]
mod ahead;
#[cfg(test)]
#[path = "game/mobs/allay_jukebox_source_tests.rs"]
mod allay_jukebox_source_tests;
#[cfg(test)]
#[path = "game/mobs/allay_pickup_cooldown_tests.rs"]
mod allay_pickup_cooldown_tests;
#[path = "game/mobs/allays.rs"]
mod allays;
#[path = "game/modding/api_values.rs"]
pub(crate) mod api_values;
#[cfg(test)]
#[path = "game/entities/armor_stand_refind_tests.rs"]
mod armor_stand_refind_tests;
#[path = "game/entities/armor_stands.rs"]
mod armor_stands;
#[path = "game/interface/atlas_slots.rs"]
mod atlas_slots;
#[path = "game/modding/attachment_index/mod.rs"]
mod attachment_index;
#[cfg(test)]
#[path = "game/modding/attachment_instruction_bench.rs"]
mod attachment_instruction_bench;
#[path = "game/modding/attachment_sync.rs"]
mod attachment_sync;
#[path = "game/world/autosave.rs"]
mod autosave;
#[cfg(test)]
#[path = "game/modding/baked_model_keys_tests.rs"]
mod baked_model_keys_tests;
#[path = "game/blocks/banners.rs"]
mod banners;
#[cfg(test)]
#[path = "game/modding/bare_declaration_names_tests.rs"]
mod bare_declaration_names_tests;
#[path = "game/blocks/beacon.rs"]
mod beacon;
#[cfg(test)]
#[path = "game/blocks/beacon_cull_tests.rs"]
mod beacon_cull_tests;
#[path = "game/blocks/beds.rs"]
mod beds;
#[path = "game/mobs/beehives.rs"]
mod beehives;
#[path = "game/blocks/block_breaking.rs"]
mod block_breaking;
#[cfg(test)]
#[path = "game/blocks/block_drops_pops_tests.rs"]
mod block_drops_pops_tests;
#[path = "game/interface/block_editor.rs"]
mod block_editor;
#[path = "game/blocks/block_edits.rs"]
mod block_edits;
#[path = "game/blocks/block_entity_anims.rs"]
mod block_entity_anims;
#[cfg(test)]
#[path = "game/modding/block_entity_part_state_tests.rs"]
mod block_entity_part_state_tests;
#[cfg(test)]
#[path = "game/blocks/block_entity_reach_draw_tests.rs"]
pub(in crate::game) mod block_entity_reach_draw_tests;
#[path = "game/blocks/block_entity_ticks.rs"]
mod block_entity_ticks;
#[path = "game/blocks/block_items.rs"]
mod block_items;
#[path = "game/modding/block_notifications.rs"]
mod block_notifications;
#[cfg(test)]
#[path = "game/blocks/block_part_tool_tests.rs"]
mod block_part_tool_tests;
#[path = "game/blocks/block_particles.rs"]
mod block_particles;
#[cfg(test)]
#[path = "game/interface/block_screen_effect_draw_tests.rs"]
mod block_screen_effect_draw_tests;
#[path = "game/blocks/block_ticks.rs"]
mod block_ticks;
#[path = "game/modding/block_use_completion.rs"]
mod block_use_completion;
#[cfg(test)]
#[path = "game/modding/block_use_completion_tests.rs"]
mod block_use_completion_tests;
#[path = "game/runtime/blockctl.rs"]
mod blockctl;
#[path = "game/entities/boarding.rs"]
mod boarding;
#[path = "game/entities/boat_authority_fixture.rs"]
mod boat_authority_fixture;
#[path = "game/entities/boat_leads.rs"]
mod boat_leads;
#[path = "game/entities/boat_passengers.rs"]
mod boat_passengers;
#[cfg(test)]
#[path = "game/entities/boat_refind_tests.rs"]
mod boat_refind_tests;
#[path = "game/entities/boat_rider_fixture.rs"]
mod boat_rider_fixture;
#[path = "game/entities/boats/mod.rs"]
mod boats;
#[path = "game/interface/books.rs"]
mod books;
#[path = "game/blocks/bookshelves.rs"]
mod bookshelves;
#[path = "game/interface/bossbars.rs"]
mod bossbars;
#[path = "game/mobs/breezes.rs"]
mod breezes;
#[path = "game/blocks/brewing_remainder.rs"]
mod brewing_remainder;
#[path = "game/blocks/brush.rs"]
mod brush;
#[path = "game/items/bucket_empty.rs"]
mod bucket_empty;
#[path = "game/items/buckets.rs"]
mod buckets;
#[path = "game/interface/building_players.rs"]
mod building_players;
#[cfg(test)]
#[path = "game/items/bundle_use_tests.rs"]
mod bundle_use_tests;
#[path = "game/items/bundles.rs"]
mod bundles;
#[path = "game/items/cake_eligibility.rs"]
mod cake_eligibility;
#[path = "game/interface/camera_hud.rs"]
mod camera_hud;
#[cfg(test)]
#[path = "game/interface/camera_submersion_tests.rs"]
mod camera_submersion_tests;
#[path = "game/blocks/campfire.rs"]
mod campfire;
#[path = "game/blocks/campfire_douse.rs"]
mod campfire_douse;
#[path = "game/world/carrier_transfers.rs"]
mod carrier_transfers;
#[path = "game/entities/cart_rails.rs"]
mod cart_rails;
#[path = "game/blocks/cartography.rs"]
mod cartography;
#[path = "game/mobs/cat_spawner.rs"]
mod cat_spawner;
#[path = "game/blocks/cauldron_interactions.rs"]
mod cauldron_interactions;
#[path = "game/network/cauldron_prediction.rs"]
mod cauldron_prediction;
#[cfg(test)]
#[path = "game/blocks/cauldron_underwater_tests.rs"]
mod cauldron_underwater_tests;
#[path = "game/blocks/cauldrons.rs"]
mod cauldrons;
#[path = "game/interface/chat.rs"]
mod chat;
#[path = "game/blocks/chiseled_bookshelf.rs"]
mod chiseled_bookshelf;
#[path = "game/world/class_tags.rs"]
mod class_tags;
#[cfg(test)]
#[path = "game/world/class_tags_tests.rs"]
mod class_tags_tests;
#[path = "game/interface/client_ambient.rs"]
mod client_ambient;
#[path = "game/items/client_attack_targets.rs"]
mod client_attack_targets;
#[path = "game/modding/client_cracks.rs"]
mod client_cracks;
#[path = "game/modding/client_executor.rs"]
mod client_executor;
#[path = "game/modding/client_item_extensions.rs"]
mod client_item_extensions;
#[path = "game/interface/client_reload/mod.rs"]
mod client_reload;
#[path = "game/runtime/client_tick_steps.rs"]
mod client_tick_steps;
#[path = "game/items/clip_ray.rs"]
mod clip_ray;
#[path = "game/interface/clipboard.rs"]
mod clipboard;
#[cfg(test)]
#[path = "game/blocks/clone_block_ticks_tests.rs"]
mod clone_block_ticks_tests;
#[cfg(test)]
#[path = "game/administration/clone_dimensions_tests.rs"]
mod clone_dimensions_tests;
#[path = "game/entities/closest_space.rs"]
mod closest_space;
#[cfg(test)]
#[path = "game/interface/cloud_height_frame_tests.rs"]
mod cloud_height_frame_tests;
#[path = "game/interface/code_of_conduct.rs"]
mod code_of_conduct;
#[path = "game/entities/collision_source.rs"]
mod collision_source;
#[path = "game/items/combat.rs"]
mod combat;
#[path = "game/administration/command_blocks.rs"]
mod command_blocks;
#[path = "game/administration/command_harness.rs"]
mod command_harness;
#[path = "game/administration/command_minecarts.rs"]
mod command_minecarts;
#[path = "game/interface/command_registries.rs"]
mod command_registries;
#[path = "game/administration/commands/mod.rs"]
mod commands;
#[path = "game/blocks/composter.rs"]
mod composter;
#[path = "game/world/condition_reload_state.rs"]
mod condition_reload_state;
#[path = "game/items/consume.rs"]
mod consume;
#[cfg(test)]
#[path = "game/modding/consume_effect_dispatch_tests.rs"]
mod consume_effect_dispatch_tests;
#[cfg(test)]
#[path = "game/interface/contained_fluid_tests.rs"]
mod contained_fluid_tests;
#[path = "game/interface/container_hooks.rs"]
mod container_hooks;
#[path = "game/interface/container_layout.rs"]
mod container_layout;
#[path = "game/world/container_loot.rs"]
mod container_loot;
#[cfg(test)]
#[path = "game/items/continue_using_tests.rs"]
mod continue_using_tests;
#[path = "game/runtime/control.rs"]
mod control;
#[cfg(test)]
#[path = "game/mobs/conversion_place_tests.rs"]
mod conversion_place_tests;
#[path = "game/mobs/copper_golems.rs"]
mod copper_golems;
#[cfg(test)]
#[path = "game/runtime/core_tests.rs"]
mod core_tests;
#[cfg(test)]
#[path = "game/modding/crafting_recipe_kind_tests.rs"]
mod crafting_recipe_kind_tests;
#[path = "game/mobs/creaking.rs"]
mod creaking;
#[cfg(test)]
#[path = "game/interface/create_fluid_source_tests.rs"]
mod create_fluid_source_tests;
#[path = "game/items/creative_hotbar.rs"]
mod creative_hotbar;
#[path = "game/interface/creative_screen.rs"]
mod creative_screen;
#[cfg(test)]
#[path = "game/items/creator_mod_id_tests.rs"]
mod creator_mod_id_tests;
#[path = "game/mobs/creatures/mod.rs"]
mod creatures;
#[cfg(test)]
#[path = "game/interface/crop_grow_tests.rs"]
mod crop_grow_tests;
#[path = "game/items/crossbow.rs"]
mod crossbow;
#[cfg(test)]
#[path = "game/entities/crosshair_pick_tests.rs"]
mod crosshair_pick_tests;
#[cfg(test)]
#[path = "game/entities/cushion_refind_tests.rs"]
mod cushion_refind_tests;
#[path = "game/entities/cushions.rs"]
pub(crate) mod cushions;
#[path = "game/interface/custom_arm_pose.rs"]
mod custom_arm_pose;
#[path = "game/interface/custom_sky.rs"]
mod custom_sky;
#[path = "game/mobs/custom_spawner_level.rs"]
mod custom_spawner_level;
#[path = "game/mobs/custom_spawners.rs"]
mod custom_spawners;
#[cfg(test)]
#[path = "game/items/damage_item_tests.rs"]
mod damage_item_tests;
#[cfg(test)]
#[path = "game/items/damage_resistant_tests.rs"]
mod damage_resistant_tests;
#[path = "game/modding/data_map_files.rs"]
mod data_map_files;
#[path = "game/modding/data_map_json.rs"]
mod data_map_json;
#[path = "game/interface/debug_keys.rs"]
mod debug_keys;
#[path = "game/interface/debug_lines.rs"]
mod debug_lines;
#[path = "game/interface/debug_options.rs"]
mod debug_options;
#[cfg(test)]
#[path = "game/modding/debug_renderer_tests.rs"]
mod debug_renderer_tests;
#[path = "game/interface/debug_renderers.rs"]
mod debug_renderers;
#[path = "game/blocks/debug_stick.rs"]
mod debug_stick;
#[path = "game/modding/decided_action.rs"]
mod decided_action;
#[cfg(test)]
#[path = "game/modding/declared_vegetation_tests.rs"]
mod declared_vegetation_tests;
#[path = "game/blocks/decorated_pot.rs"]
mod decorated_pot;
#[path = "game/interface/dedicated/mod.rs"]
pub(crate) mod dedicated;
#[path = "game/items/deflect.rs"]
mod deflect;
#[path = "game/interface/demo_intro.rs"]
mod demo_intro;
#[path = "game/interface/developer_launch.rs"]
mod developer_launch;
#[path = "game/interface/dialogs.rs"]
mod dialogs;
#[cfg(test)]
#[path = "game/modding/disabled_block_tests.rs"]
mod disabled_block_tests;
#[path = "game/blocks/dispense_bucket.rs"]
mod dispense_bucket;
#[cfg(test)]
#[path = "game/blocks/dispense_container_tests.rs"]
mod dispense_container_tests;
#[path = "game/interface/dispense_fixture.rs"]
mod dispense_fixture;
#[path = "game/blocks/dispenser.rs"]
mod dispenser;
#[path = "game/mobs/dragon.rs"]
mod dragon;
#[cfg(test)]
#[path = "game/interface/dragon_boss_bar_name_tests.rs"]
mod dragon_boss_bar_name_tests;
#[cfg(test)]
#[path = "game/mobs/dragon_fireball_motion_tests.rs"]
mod dragon_fireball_motion_tests;
#[cfg(test)]
#[path = "game/items/dropped_by_player_tests.rs"]
mod dropped_by_player_tests;
#[path = "game/modding/dropped_item_setter.rs"]
mod dropped_item_setter;
#[path = "game/items/durability.rs"]
mod durability;
#[path = "game/world/dynamic_light_emission.rs"]
mod dynamic_light_emission;
#[cfg(test)]
#[path = "game/world/dynamic_light_emission_tests.rs"]
mod dynamic_light_emission_tests;
#[path = "game/interface/dynamic_lights.rs"]
mod dynamic_lights;
#[path = "game/world/early_game_rules.rs"]
mod early_game_rules;
#[cfg(test)]
#[path = "game/world/early_game_rules_tests.rs"]
mod early_game_rules_tests;
#[cfg(test)]
#[path = "game/blocks/empty_contents_tests.rs"]
mod empty_contents_tests;
#[path = "game/items/enchant_effects.rs"]
mod enchant_effects;
#[cfg(test)]
#[path = "game/items/enchant_pick_parts_tests.rs"]
mod enchant_pick_parts_tests;
#[cfg(test)]
#[path = "game/blocks/enchant_power_beam_tests.rs"]
mod enchant_power_beam_tests;
#[path = "game/world/end.rs"]
mod end;
#[path = "game/mobs/enderman.rs"]
mod enderman;
#[cfg(test)]
#[path = "game/mobs/enderman_hurt_teleport_tests.rs"]
mod enderman_hurt_teleport_tests;
#[cfg(test)]
#[path = "game/interface/enderman_placement_tests.rs"]
mod enderman_placement_tests;
#[path = "game/entities/entity.rs"]
mod entity;
#[path = "game/interface/entity_culling/mod.rs"]
mod entity_culling;
#[cfg(test)]
#[path = "game/entities/entity_die_tests.rs"]
mod entity_die_tests;
#[path = "game/modding/entity_initialization.rs"]
mod entity_initialization;
#[cfg(test)]
#[path = "game/entities/entity_inside_tests.rs"]
mod entity_inside_tests;
#[cfg(test)]
#[path = "game/interface/entity_model_loading_tests.rs"]
mod entity_model_loading_tests;
#[path = "game/modding/entity_part_state.rs"]
mod entity_part_state;
#[cfg(test)]
#[path = "game/modding/entity_part_state_tests.rs"]
mod entity_part_state_tests;
#[path = "game/entities/entity_residency.rs"]
mod entity_residency;
#[path = "game/modding/entity_sections.rs"]
mod entity_sections;
#[path = "game/modding/entity_sizes.rs"]
mod entity_sizes;
#[path = "game/entities/entity_sounds.rs"]
mod entity_sounds;
#[path = "game/modding/entity_type_rows.rs"]
mod entity_type_rows;
#[path = "game/items/equip_answers.rs"]
mod equip_answers;
#[path = "game/entities/equipment_piece.rs"]
mod equipment_piece;
#[cfg(test)]
#[path = "game/mobs/equipment_tick_tests.rs"]
mod equipment_tick_tests;
#[path = "game/runtime/event_loop.rs"]
mod event_loop;
#[path = "game/runtime/events.rs"]
mod events;
#[path = "game/runtime/execution_role.rs"]
mod execution_role;
#[cfg(test)]
#[path = "game/interface/experience_points_tests.rs"]
mod experience_points_tests;
#[path = "game/entities/explosion.rs"]
mod explosion;
#[path = "game/entities/explosion_detonate.rs"]
mod explosion_detonate;
#[path = "game/entities/explosion_settings.rs"]
mod explosion_settings;
#[path = "game/entities/extinguish.rs"]
mod extinguish;
#[path = "game/interface/extras.rs"]
mod extras;
#[cfg(test)]
#[path = "game/runtime/falling_air_tick_tests.rs"]
mod falling_air_tick_tests;
#[path = "game/blocks/falling_blocks.rs"]
mod falling_blocks;
#[path = "game/entities/falling_damage.rs"]
mod falling_damage;
#[path = "game/entities/falling_save.rs"]
mod falling_save;
#[path = "game/entities/falling_tick.rs"]
mod falling_tick;
#[cfg(test)]
#[path = "game/entities/falling_tick_tests.rs"]
mod falling_tick_tests;
#[path = "game/mobs/fangs.rs"]
mod fangs;
#[path = "game/world/far_terrain_edits.rs"]
mod far_terrain_edits;
#[cfg(test)]
#[path = "game/mobs/fence_leash_force_tests.rs"]
mod fence_leash_force_tests;
#[cfg(test)]
#[path = "game/blocks/fertile_part_tests.rs"]
mod fertile_part_tests;
#[path = "game/mobs/finalize_spawn.rs"]
mod finalize_spawn;
#[path = "game/entities/fireworks.rs"]
mod fireworks;
#[cfg(test)]
#[path = "game/blocks/first_tick_registration_tests.rs"]
mod first_tick_registration_tests;
#[path = "game/items/fishing.rs"]
mod fishing;
#[cfg(test)]
#[path = "game/interface/fluid_interaction_tests.rs"]
mod fluid_interaction_tests;
#[path = "game/modding/fluid_interactions.rs"]
mod fluid_interactions;
#[cfg(test)]
#[path = "game/interface/fluid_passage_tests.rs"]
mod fluid_passage_tests;
#[path = "game/interface/focused_text_field.rs"]
mod focused_text_field;
#[path = "game/interface/fov.rs"]
mod fov;
#[path = "game/mobs/foxes.rs"]
mod foxes;
#[path = "game/interface/frame_geometry.rs"]
mod frame_geometry;
#[cfg(test)]
#[path = "game/modding/frame_view_retained_draw_tests.rs"]
mod frame_view_retained_draw_tests;
#[path = "game/entities/frames.rs"]
mod frames;
#[cfg(feature = "friends")]
#[path = "game/network/friends.rs"]
mod friends;
#[cfg(not(feature = "friends"))]
#[path = "game/network/friends_off.rs"]
mod friends;
#[path = "game/administration/functions.rs"]
mod functions;
#[path = "game/interface/game_mode.rs"]
mod game_mode;
#[path = "game/administration/game_tests.rs"]
mod game_tests;
#[path = "game/interface/gamemode_switcher.rs"]
mod gamemode_switcher;
#[path = "game/entities/general.rs"]
mod general;
#[path = "game/mobs/general_draw.rs"]
mod general_draw;
#[path = "game/mobs/ghasts.rs"]
mod ghasts;
#[path = "game/items/glide_wear.rs"]
mod glide_wear;
#[cfg(test)]
#[path = "game/items/glide_wear_tests.rs"]
mod glide_wear_tests;
#[path = "game/modding/global_loot_modifiers.rs"]
mod global_loot_modifiers;
#[path = "game/entities/glowing.rs"]
mod glowing;
#[cfg(test)]
#[path = "game/mobs/golem_defense_tests.rs"]
mod golem_defense_tests;
#[path = "game/mobs/golems.rs"]
mod golems;
#[cfg(test)]
#[path = "game/modding/grindstone_decisions_tests.rs"]
mod grindstone_decisions_tests;
#[path = "game/blocks/growth.rs"]
mod growth;
#[cfg(test)]
#[path = "game/modding/gui_canvas_items_tests.rs"]
mod gui_canvas_items_tests;
#[path = "game/modding/gui_clips.rs"]
mod gui_clips;
#[path = "game/interface/gui_entity.rs"]
mod gui_entity;
#[cfg(test)]
#[path = "game/modding/gui_gradient_tests.rs"]
mod gui_gradient_tests;
#[cfg(test)]
#[path = "game/modding/gui_layer_tests.rs"]
mod gui_layer_tests;
#[path = "game/items/hand.rs"]
mod hand;
#[cfg(test)]
#[path = "game/items/hand_part_tests.rs"]
mod hand_part_tests;
#[cfg(test)]
#[path = "game/entities/hanging_refind_tests.rs"]
mod hanging_refind_tests;
#[path = "game/runtime/headless.rs"]
mod headless;
#[path = "game/items/held_items.rs"]
mod held_items;
#[cfg(test)]
#[path = "game/interface/highlight_tip_tests.rs"]
mod highlight_tip_tests;
#[path = "game/interface/hit.rs"]
mod hit;
#[cfg(test)]
#[path = "game/mobs/hoglin_repeat_attack_tests.rs"]
mod hoglin_repeat_attack_tests;
#[cfg(test)]
#[path = "game/modding/hook_meddler_tests.rs"]
mod hook_meddler_tests;
mod host;
#[path = "game/runtime/hot_paths/mod.rs"]
mod hot_paths;
#[path = "game/modding/hotbar_items.rs"]
mod hotbar_items;
#[path = "game/interface/hud_editor.rs"]
mod hud_editor;
#[path = "game/interface/hud_render.rs"]
mod hud_render;
#[path = "game/interface/hud_room.rs"]
mod hud_room;
#[path = "game/interface/humanoid_setup_anim.rs"]
mod humanoid_setup_anim;
#[path = "game/interface/ime.rs"]
mod ime;
#[path = "game/world/infiniburn.rs"]
mod infiniburn;
#[cfg(test)]
#[path = "game/world/infiniburn_tests.rs"]
mod infiniburn_tests;
#[path = "game/modding/initial_client_models.rs"]
mod initial_client_models;
#[cfg(test)]
#[path = "game/modding/initial_client_models_tests.rs"]
mod initial_client_models_tests;
#[path = "game/world/initial_spawn.rs"]
mod initial_spawn;
#[cfg(test)]
#[path = "game/administration/inline_feature_parse_tests.rs"]
mod inline_feature_parse_tests;
#[cfg(test)]
#[path = "game/administration/inline_loot_reference_tests.rs"]
mod inline_loot_reference_tests;
#[path = "game/runtime/input_trace.rs"]
mod input_trace;
#[path = "game/world/integrated.rs"]
mod integrated;
#[cfg(test)]
#[path = "game/entities/interaction_data_tests.rs"]
mod interaction_data_tests;
#[path = "game/items/interactions.rs"]
mod interactions;
#[path = "game/interface/inventory_input.rs"]
mod inventory_input;
#[path = "game/interface/inventory_slots.rs"]
mod inventory_slots;
#[path = "game/items/inventory_tick.rs"]
mod inventory_tick;
#[cfg(test)]
#[path = "game/mobs/invulnerable_time_tests.rs"]
mod invulnerable_time_tests;
#[path = "game/items/item_abilities.rs"]
mod item_abilities;
#[cfg(test)]
#[path = "game/items/item_abilities_tests.rs"]
mod item_abilities_tests;
#[path = "game/items/item_actions.rs"]
mod item_actions;
#[cfg(test)]
#[path = "game/items/item_attack_tests.rs"]
mod item_attack_tests;
#[cfg(test)]
#[path = "game/entities/item_contact_tests.rs"]
mod item_contact_tests;
#[path = "game/interface/item_draw.rs"]
mod item_draw;
#[cfg(test)]
#[path = "game/modding/item_drop_state_tests.rs"]
mod item_drop_state_tests;
#[path = "game/items/item_durability.rs"]
mod item_durability;
#[path = "game/items/item_entities.rs"]
mod item_entities;
#[path = "game/items/item_entity_spawn.rs"]
mod item_entity_spawn;
#[path = "game/entities/item_hurt.rs"]
mod item_hurt;
#[cfg(test)]
#[path = "game/items/item_interact_mob_tests.rs"]
mod item_interact_mob_tests;
#[cfg(test)]
#[path = "game/items/item_menu_parts_tests.rs"]
mod item_menu_parts_tests;
#[path = "game/modding/item_model_context.rs"]
mod item_model_context;
#[cfg(test)]
#[path = "game/items/item_name_tests.rs"]
mod item_name_tests;
#[path = "game/modding/item_particle_send.rs"]
mod item_particle_send;
#[cfg(test)]
#[path = "game/modding/item_particle_send_tests.rs"]
mod item_particle_send_tests;
#[path = "game/items/item_particles.rs"]
mod item_particles;
#[cfg(test)]
#[path = "game/items/item_pickup_shape_tests.rs"]
mod item_pickup_shape_tests;
#[cfg(test)]
#[path = "game/interface/item_slot_command_tests.rs"]
mod item_slot_command_tests;
#[path = "game/items/item_use.rs"]
mod item_use;
#[path = "game/items/item_use_stacks.rs"]
mod item_use_stacks;
#[cfg(test)]
#[path = "game/entities/item_void_tests.rs"]
mod item_void_tests;
#[path = "game/administration/jigsaw_blocks.rs"]
mod jigsaw_blocks;
#[path = "game/administration/jigsaw_edit.rs"]
mod jigsaw_edit;
#[path = "game/modding/journal_level_write.rs"]
mod journal_level_write;
#[path = "game/blocks/jukebox.rs"]
mod jukebox;
#[path = "game/interface/key_codes.rs"]
mod key_codes;
#[path = "game/interface/key_focus.rs"]
mod key_focus;
#[cfg(test)]
#[path = "game/interface/keymap_tests.rs"]
mod keymap_tests;
#[cfg(test)]
#[path = "game/entities/kill_die_cancel_tests.rs"]
mod kill_die_cancel_tests;
#[cfg(test)]
#[path = "game/entities/kill_die_vibration_tests.rs"]
mod kill_die_vibration_tests;
#[cfg(test)]
#[path = "game/entities/knot_residency_tests.rs"]
mod knot_residency_tests;
#[path = "game/network/lan/mod.rs"]
mod lan;
#[path = "game/network/lan_actions/mod.rs"]
mod lan_actions;
#[path = "game/network/lan_attributes.rs"]
mod lan_attributes;
#[path = "game/network/lan_camera.rs"]
mod lan_camera;
#[path = "game/network/lan_chat.rs"]
mod lan_chat;
#[path = "game/network/lan_configuration_tasks.rs"]
mod lan_configuration_tasks;
#[path = "game/network/lan_crafting.rs"]
mod lan_crafting;
#[path = "game/network/lan_entities.rs"]
mod lan_entities;
#[path = "game/network/lan_far.rs"]
mod lan_far;
#[path = "game/network/lan_menus.rs"]
mod lan_menus;
#[cfg(test)]
#[path = "game/network/lan_menus_tests.rs"]
mod lan_menus_tests;
#[path = "game/network/lan_mobs.rs"]
mod lan_mobs;
#[path = "game/network/lan_mount_menu.rs"]
mod lan_mount_menu;
#[path = "game/administration/lan_op.rs"]
mod lan_op;
#[path = "game/network/lan_particles.rs"]
mod lan_particles;
#[path = "game/network/lan_pickup.rs"]
mod lan_pickup;
#[path = "game/network/lan_pvp.rs"]
mod lan_pvp;
#[path = "game/network/lan_quiet.rs"]
mod lan_quiet;
#[path = "game/network/lan_screen.rs"]
mod lan_screen;
#[path = "game/network/lan_stations.rs"]
mod lan_stations;
#[path = "game/network/lan_trade_decisions.rs"]
mod lan_trade_decisions;
#[cfg(test)]
#[path = "game/network/lan_trade_decisions_tests.rs"]
mod lan_trade_decisions_tests;
#[path = "game/network/lan_trading.rs"]
mod lan_trading;
#[path = "game/network/lan_vehicles.rs"]
mod lan_vehicles;
#[path = "game/network/lan_waypoints.rs"]
mod lan_waypoints;
#[cfg(test)]
#[path = "game/mobs/last_hurt_by_mob_tests.rs"]
mod last_hurt_by_mob_tests;
#[path = "game/runtime/launch_api.rs"]
mod launch_api;
#[path = "game/runtime/launch_options.rs"]
mod launch_options;
#[path = "game/runtime/launch_timeout.rs"]
mod launch_timeout;
#[path = "game/entities/leads.rs"]
mod leads;
#[path = "game/blocks/level_access.rs"]
mod level_access;
#[path = "game/interface/level_particles.rs"]
mod level_particles;
#[path = "game/interface/level_sounds.rs"]
mod level_sounds;
#[path = "game/world/level_tick.rs"]
mod level_tick;
#[path = "game/world/levels.rs"]
mod levels;
#[cfg(test)]
#[path = "game/interface/lightmap_expression_tests.rs"]
mod lightmap_expression_tests;
#[path = "game/entities/lightning.rs"]
mod lightning;
#[cfg(test)]
#[path = "game/entities/lightning_bolt_uniform_tests.rs"]
mod lightning_bolt_uniform_tests;
#[cfg(test)]
#[path = "game/entities/lightning_item_tests.rs"]
mod lightning_item_tests;
#[path = "game/mobs/living_attributes.rs"]
mod living_attributes;
#[path = "game/mobs/llamas.rs"]
mod llamas;
#[path = "game/modding/load_conditions.rs"]
mod load_conditions;
#[path = "game/runtime/loading_screen.rs"]
mod loading_screen;
#[path = "game/runtime/loading_window.rs"]
mod loading_window;
#[path = "game/blocks/location_world.rs"]
mod location_world;
#[path = "game/interface/locator_bar.rs"]
mod locator_bar;
#[path = "game/interface/look.rs"]
mod look;
#[path = "game/blocks/loom.rs"]
mod loom;
#[cfg(test)]
#[path = "game/administration/loot_kill_guest_tests.rs"]
mod loot_kill_guest_tests;
#[path = "game/mobs/loot_rolls.rs"]
mod loot_rolls;
#[path = "game/modding/loot_tags.rs"]
mod loot_tags;
#[path = "game/modding/loot_types.rs"]
pub(crate) mod loot_types;
#[path = "game/items/mace.rs"]
mod mace;
#[path = "game/blocks/machines/mod.rs"]
mod machines;
#[cfg(test)]
#[path = "game/entities/mannequin_save_tests.rs"]
mod mannequin_save_tests;
#[path = "game/blocks/maps/mod.rs"]
mod maps;
#[path = "game/entities/melee.rs"]
mod melee;
#[path = "game/runtime/memory.rs"]
mod memory;
#[cfg(test)]
#[path = "game/items/mending_pick_tests.rs"]
mod mending_pick_tests;
#[path = "game/interface/menu_clicks.rs"]
mod menu_clicks;
#[path = "game/entities/minecart_moves.rs"]
mod minecart_moves;
#[path = "game/entities/minecarts.rs"]
mod minecarts;
#[cfg(test)]
#[path = "game/runtime/mining_component_tests.rs"]
mod mining_component_tests;
#[path = "game/mobs/mob_blocks.rs"]
mod mob_blocks;
#[cfg(test)]
#[path = "game/mobs/mob_discard_tests.rs"]
mod mob_discard_tests;
#[path = "game/mobs/mob_gear.rs"]
mod mob_gear;
#[cfg(test)]
#[path = "game/world/mob_hurt_cooldown_tests.rs"]
mod mob_hurt_cooldown_tests;
#[cfg(test)]
#[path = "game/mobs/mob_landing_support_tests.rs"]
mod mob_landing_support_tests;
#[path = "game/mobs/mob_level.rs"]
mod mob_level;
#[path = "game/mobs/mob_light.rs"]
mod mob_light;
#[path = "game/mobs/mob_looting.rs"]
mod mob_looting;
#[path = "game/mobs/mob_sounds.rs"]
mod mob_sounds;
#[path = "game/modding/mod_api_host.rs"]
mod mod_api_host;
#[path = "game/modding/mod_container_openers.rs"]
mod mod_container_openers;
#[cfg(test)]
#[path = "game/modding/mod_container_openers_tests.rs"]
mod mod_container_openers_tests;
#[path = "game/modding/mod_crafting_commands.rs"]
mod mod_crafting_commands;
#[cfg(test)]
#[path = "game/modding/mod_menu_decisions_tests.rs"]
mod mod_menu_decisions_tests;
#[cfg(test)]
#[path = "game/modding/mod_raycast_tests.rs"]
mod mod_raycast_tests;
#[cfg(test)]
#[path = "game/interface/mod_recipe_property_tests.rs"]
mod mod_recipe_property_tests;
#[path = "game/interface/mod_toasts.rs"]
mod mod_toasts;
#[cfg(test)]
#[path = "game/modding/model_baking_completed_tests.rs"]
mod model_baking_completed_tests;
#[path = "game/modding/mods/mod.rs"]
pub(crate) mod mods;
#[path = "game/blocks/motion_ids.rs"]
mod motion_ids;
#[path = "game/interface/mount_menu.rs"]
mod mount_menu;
#[path = "game/mobs/mounts.rs"]
mod mounts;
#[path = "game/entities/move_gate.rs"]
mod move_gate;
#[path = "game/blocks/moving_pistons.rs"]
mod moving_pistons;
#[path = "game/interface/music.rs"]
mod music;
#[path = "game/interface/names.rs"]
mod names;
#[path = "game/interface/narrator.rs"]
mod narrator;
#[path = "game/modding/native_ai_pause.rs"]
mod native_ai_pause;
#[path = "game/modding/native_audio_streams.rs"]
mod native_audio_streams;
#[path = "game/modding/native_biome_colors.rs"]
mod native_biome_colors;
#[path = "game/modding/native_biome_keys.rs"]
mod native_biome_keys;
#[cfg(test)]
#[path = "game/modding/native_biome_keys_tests.rs"]
mod native_biome_keys_tests;
#[path = "game/modding/native_biome_source.rs"]
mod native_biome_source;
#[cfg(test)]
#[path = "game/modding/native_biome_tag_tests.rs"]
mod native_biome_tag_tests;
#[cfg(test)]
#[path = "game/modding/native_block_class_tests.rs"]
mod native_block_class_tests;
#[path = "game/modding/native_block_entity_draw.rs"]
pub mod native_block_entity_draw;
#[cfg(test)]
#[path = "game/modding/native_block_entity_item_tests.rs"]
mod native_block_entity_item_tests;
#[path = "game/modding/native_block_entity_refs.rs"]
mod native_block_entity_refs;
#[path = "game/modding/native_block_entity_transform.rs"]
mod native_block_entity_transform;
#[cfg(test)]
#[path = "game/modding/native_block_entity_transform_tests.rs"]
mod native_block_entity_transform_tests;
#[path = "game/modding/native_block_extra_models.rs"]
mod native_block_extra_models;
#[path = "game/modding/native_block_sounds.rs"]
pub(crate) mod native_block_sounds;
#[path = "game/modding/native_block_sprites.rs"]
mod native_block_sprites;
#[path = "game/modding/native_block_states.rs"]
mod native_block_states;
#[path = "game/modding/native_block_tables.rs"]
mod native_block_tables;
#[cfg(test)]
#[path = "game/modding/native_block_tables_tests.rs"]
mod native_block_tables_tests;
#[cfg(test)]
#[path = "game/modding/native_block_use_criteria_tests.rs"]
mod native_block_use_criteria_tests;
#[path = "game/modding/native_book_apply.rs"]
mod native_book_apply;
#[path = "game/modding/native_book_catalogue.rs"]
mod native_book_catalogue;
#[path = "game/modding/native_book_displays.rs"]
mod native_book_displays;
#[path = "game/modding/native_book_grid.rs"]
mod native_book_grid;
#[path = "game/modding/native_book_invalidation.rs"]
mod native_book_invalidation;
#[path = "game/modding/native_book_net.rs"]
mod native_book_net;
#[path = "game/modding/native_book_placement.rs"]
mod native_book_placement;
#[path = "game/modding/native_book_receive.rs"]
mod native_book_receive;
#[path = "game/modding/native_book_settings.rs"]
mod native_book_settings;
#[path = "game/modding/native_book_transport.rs"]
mod native_book_transport;
#[path = "game/modding/native_boss_bar_refs.rs"]
mod native_boss_bar_refs;
#[path = "game/modding/native_bots.rs"]
mod native_bots;
#[cfg(test)]
#[path = "game/modding/native_capability_cache_tests.rs"]
mod native_capability_cache_tests;
#[path = "game/modding/native_cauldron_contents.rs"]
mod native_cauldron_contents;
#[cfg(test)]
#[path = "game/modding/native_chat_recipients_tests.rs"]
mod native_chat_recipients_tests;
#[path = "game/modding/native_chunk_areas.rs"]
mod native_chunk_areas;
#[path = "game/modding/native_chunk_generator.rs"]
mod native_chunk_generator;
#[path = "game/modding/native_chunk_queries.rs"]
mod native_chunk_queries;
#[cfg(test)]
#[path = "game/modding/native_chunk_relight_tests.rs"]
mod native_chunk_relight_tests;
#[path = "game/modding/native_chunk_requests.rs"]
mod native_chunk_requests;
#[path = "game/modding/native_chunk_save.rs"]
mod native_chunk_save;
#[path = "game/modding/native_chunk_snapshot.rs"]
mod native_chunk_snapshot;
#[path = "game/modding/native_claimed_items.rs"]
mod native_claimed_items;
#[path = "game/modding/native_client_particles.rs"]
mod native_client_particles;
#[path = "game/modding/native_client_particles_fixture.rs"]
mod native_client_particles_fixture;
#[path = "game/interface/native_client_screens.rs"]
mod native_client_screens;
#[path = "game/modding/native_client_services.rs"]
pub mod native_client_services;
#[path = "game/modding/native_client_session.rs"]
mod native_client_session;
#[path = "game/modding/native_colliders.rs"]
mod native_colliders;
#[cfg(test)]
#[path = "game/modding/native_colliders_tests.rs"]
mod native_colliders_tests;
#[path = "game/modding/native_collision_exclusions.rs"]
mod native_collision_exclusions;
#[path = "game/modding/native_collision_shape.rs"]
pub(crate) mod native_collision_shape;
#[cfg(test)]
#[path = "game/modding/native_command_completion_tests.rs"]
mod native_command_completion_tests;
#[cfg(test)]
#[path = "game/modding/native_command_entity_tests.rs"]
mod native_command_entity_tests;
#[path = "game/modding/native_command_reads.rs"]
mod native_command_reads;
#[path = "game/modding/native_components.rs"]
mod native_components;
#[path = "game/modding/native_configs.rs"]
mod native_configs;
#[path = "game/modding/native_connection_groups.rs"]
mod native_connection_groups;
#[path = "game/modding/native_console.rs"]
mod native_console;
#[cfg(test)]
#[path = "game/modding/native_console_tests.rs"]
mod native_console_tests;
#[path = "game/modding/native_criteria.rs"]
mod native_criteria;
#[path = "game/modding/native_custom_geometry.rs"]
mod native_custom_geometry;
#[path = "game/modding/native_custom_stats.rs"]
mod native_custom_stats;
#[cfg(test)]
#[path = "game/modding/native_damage_tests.rs"]
mod native_damage_tests;
#[path = "game/modding/native_data_bound.rs"]
mod native_data_bound;
#[path = "game/modding/native_data_maps.rs"]
mod native_data_maps;
#[path = "game/modding/native_datafix.rs"]
mod native_datafix;
#[cfg(test)]
#[path = "game/modding/native_datafix_tests.rs"]
mod native_datafix_tests;
#[path = "game/modding/native_datapack_refs.rs"]
mod native_datapack_refs;
#[path = "game/modding/native_datapack_values.rs"]
mod native_datapack_values;
#[path = "game/modding/native_death_weapon.rs"]
mod native_death_weapon;
#[path = "game/modding/native_detached_generation.rs"]
mod native_detached_generation;
#[path = "game/modding/native_dimensions.rs"]
mod native_dimensions;
#[path = "game/modding/native_disconnect.rs"]
mod native_disconnect;
#[cfg(test)]
#[path = "game/modding/native_disconnect_tests.rs"]
mod native_disconnect_tests;
#[path = "game/modding/native_display_refs.rs"]
mod native_display_refs;
#[path = "game/modding/native_dragon_refs.rs"]
mod native_dragon_refs;
#[path = "game/modding/native_dynamic_textures.rs"]
mod native_dynamic_textures;
#[cfg(test)]
#[path = "game/modding/native_dynamic_textures_tests.rs"]
mod native_dynamic_textures_tests;
#[path = "game/modding/native_effect_category.rs"]
mod native_effect_category;
#[path = "game/modding/native_entities.rs"]
mod native_entities;
#[path = "game/modding/native_entity_commands.rs"]
mod native_entity_commands;
#[path = "game/modding/native_entity_data.rs"]
mod native_entity_data;
#[path = "game/modding/native_entity_extinguish.rs"]
mod native_entity_extinguish;
#[cfg(test)]
#[path = "game/modding/native_entity_extinguish_tests.rs"]
mod native_entity_extinguish_tests;
#[cfg(test)]
#[path = "game/modding/native_entity_index_parity_tests.rs"]
mod native_entity_index_parity_tests;
#[path = "game/modding/native_entity_invulnerable.rs"]
mod native_entity_invulnerable;
#[path = "game/modding/native_entity_kind.rs"]
mod native_entity_kind;
#[path = "game/modding/native_entity_layers.rs"]
mod native_entity_layers;
#[cfg(test)]
#[path = "game/modding/native_entity_layers_tests.rs"]
mod native_entity_layers_tests;
#[path = "game/modding/native_entity_motion.rs"]
mod native_entity_motion;
#[cfg(test)]
#[path = "game/modding/native_entity_move_correction_tests.rs"]
mod native_entity_move_correction_tests;
#[cfg(test)]
#[path = "game/modding/native_entity_move_player_tests.rs"]
mod native_entity_move_player_tests;
#[cfg(test)]
#[path = "game/modding/native_entity_move_sweep_tests.rs"]
mod native_entity_move_sweep_tests;
#[cfg(test)]
#[path = "game/modding/native_entity_move_tests.rs"]
mod native_entity_move_tests;
#[path = "game/modding/native_entity_moves.rs"]
mod native_entity_moves;
#[path = "game/modding/native_entity_physics.rs"]
mod native_entity_physics;
#[path = "game/modding/native_entity_previous.rs"]
mod native_entity_previous;
#[cfg(test)]
#[path = "game/modding/native_entity_previous_tests.rs"]
mod native_entity_previous_tests;
#[path = "game/modding/native_entity_refs.rs"]
mod native_entity_refs;
#[path = "game/modding/native_entity_remove.rs"]
mod native_entity_remove;
#[path = "game/modding/native_entity_renderers.rs"]
mod native_entity_renderers;
#[cfg(test)]
#[path = "game/modding/native_entity_renderers_tests.rs"]
mod native_entity_renderers_tests;
#[path = "game/modding/native_entity_riding.rs"]
mod native_entity_riding;
#[path = "game/modding/native_entity_selectors.rs"]
mod native_entity_selectors;
#[path = "game/modding/native_entity_sounds.rs"]
mod native_entity_sounds;
#[path = "game/modding/native_entity_state.rs"]
mod native_entity_state;
#[cfg(test)]
#[path = "game/modding/native_experience_orb_renderers_tests.rs"]
mod native_experience_orb_renderers_tests;
#[path = "game/modding/native_explosion_destroy.rs"]
mod native_explosion_destroy;
#[cfg(test)]
#[path = "game/modding/native_explosion_destroy_tests.rs"]
mod native_explosion_destroy_tests;
#[path = "game/modding/native_explosion_resistance.rs"]
mod native_explosion_resistance;
#[cfg(test)]
#[path = "game/modding/native_explosion_resistance_tests.rs"]
mod native_explosion_resistance_tests;
#[path = "game/modding/native_explosions.rs"]
mod native_explosions;
#[cfg(test)]
#[path = "game/modding/native_explosions_tests.rs"]
mod native_explosions_tests;
#[cfg(test)]
#[path = "game/modding/native_fake_disconnect_tests.rs"]
mod native_fake_disconnect_tests;
#[path = "game/modding/native_falling.rs"]
mod native_falling;
#[path = "game/modding/native_feature_types.rs"]
mod native_feature_types;
#[cfg(test)]
#[path = "game/modding/native_fire_odds_tests.rs"]
mod native_fire_odds_tests;
#[path = "game/modding/native_fishing_hook_refs.rs"]
mod native_fishing_hook_refs;
#[path = "game/modding/native_fluid_render.rs"]
mod native_fluid_render;
#[path = "game/modding/native_game_event_context.rs"]
mod native_game_event_context;
#[path = "game/modding/native_game_event_keys.rs"]
mod native_game_event_keys;
#[cfg(test)]
#[path = "game/modding/native_game_event_keys_tests.rs"]
mod native_game_event_keys_tests;
#[path = "game/modding/native_game_rules_read.rs"]
mod native_game_rules_read;
#[cfg(test)]
#[path = "game/modding/native_general_renderers_tests.rs"]
mod native_general_renderers_tests;
#[path = "game/modding/native_give_stack.rs"]
mod native_give_stack;
#[cfg(test)]
#[path = "game/modding/native_give_stack_tests.rs"]
mod native_give_stack_tests;
#[path = "game/modding/native_goal_ticks/mod.rs"]
mod native_goal_ticks;
#[path = "game/modding/native_ground.rs"]
mod native_ground;
#[cfg(test)]
#[path = "game/modding/native_guest_turn_tests.rs"]
mod native_guest_turn_tests;
#[path = "game/modding/native_hanging.rs"]
mod native_hanging;
#[cfg(test)]
#[path = "game/modding/native_hanging_tests.rs"]
mod native_hanging_tests;
#[path = "game/modding/native_harvest_queries.rs"]
mod native_harvest_queries;
#[cfg(test)]
#[path = "game/modding/native_held_out_tests.rs"]
mod native_held_out_tests;
#[cfg(test)]
#[path = "game/modding/native_hit_attempt_tests.rs"]
mod native_hit_attempt_tests;
#[cfg(test)]
#[path = "game/modding/native_hit_causer_tests.rs"]
mod native_hit_causer_tests;
#[path = "game/modding/native_hit_effects.rs"]
mod native_hit_effects;
#[cfg(test)]
#[path = "game/modding/native_hit_sounds_tests.rs"]
mod native_hit_sounds_tests;
#[cfg(test)]
#[path = "game/modding/native_hit_view_tests.rs"]
mod native_hit_view_tests;
mod native_hooks;
#[path = "game/modding/native_hotbar_slot.rs"]
mod native_hotbar_slot;
#[cfg(test)]
#[path = "game/modding/native_hover_name_tests.rs"]
mod native_hover_name_tests;
#[cfg(test)]
#[path = "game/modding/native_humanoid_setup_anim_tests.rs"]
mod native_humanoid_setup_anim_tests;
#[cfg(test)]
#[path = "game/modding/native_ignited_by_lava_tests.rs"]
mod native_ignited_by_lava_tests;
#[path = "game/modding/native_instance_groups.rs"]
mod native_instance_groups;
#[path = "game/modding/native_inventories.rs"]
mod native_inventories;
#[cfg(test)]
#[path = "game/modding/native_invulnerability_check_tests.rs"]
mod native_invulnerability_check_tests;
#[cfg(test)]
#[path = "game/modding/native_is_fire_source_tests.rs"]
mod native_is_fire_source_tests;
#[cfg(test)]
#[path = "game/modding/native_is_piglin_currency_tests.rs"]
mod native_is_piglin_currency_tests;
#[path = "game/modding/native_item_bundle.rs"]
mod native_item_bundle;
#[cfg(test)]
#[path = "game/modding/native_item_bundle_tests.rs"]
mod native_item_bundle_tests;
#[path = "game/modding/native_item_container.rs"]
mod native_item_container;
#[path = "game/modding/native_item_entity_refs.rs"]
mod native_item_entity_refs;
#[cfg(test)]
#[path = "game/modding/native_item_frame_renderers_tests.rs"]
mod native_item_frame_renderers_tests;
#[path = "game/modding/native_item_menu_parts.rs"]
pub(crate) mod native_item_menu_parts;
#[path = "game/modding/native_item_particles.rs"]
mod native_item_particles;
#[path = "game/modding/native_item_places.rs"]
mod native_item_places;
#[path = "game/modding/native_item_registry.rs"]
mod native_item_registry;
#[path = "game/modding/native_key_mappings.rs"]
mod native_key_mappings;
#[cfg(test)]
#[path = "game/modding/native_key_mappings_tests.rs"]
mod native_key_mappings_tests;
#[path = "game/modding/native_knockback_credit.rs"]
mod native_knockback_credit;
#[cfg(test)]
#[path = "game/modding/native_knockback_credit_site_tests.rs"]
mod native_knockback_credit_site_tests;
#[cfg(test)]
#[path = "game/modding/native_knockback_tests.rs"]
mod native_knockback_tests;
#[cfg(test)]
#[path = "game/modding/native_level_block_states_tests.rs"]
mod native_level_block_states_tests;
#[path = "game/modding/native_level_border.rs"]
mod native_level_border;
#[path = "game/modding/native_level_effects.rs"]
mod native_level_effects;
#[path = "game/modding/native_level_events.rs"]
mod native_level_events;
#[path = "game/modding/native_level_items.rs"]
mod native_level_items;
#[cfg(test)]
#[path = "game/modding/native_level_notify_tests.rs"]
mod native_level_notify_tests;
#[cfg(test)]
#[path = "game/modding/native_level_placement_tests.rs"]
mod native_level_placement_tests;
#[path = "game/modding/native_level_players.rs"]
mod native_level_players;
#[path = "game/modding/native_level_refs.rs"]
mod native_level_refs;
#[path = "game/modding/native_level_signals.rs"]
mod native_level_signals;
#[cfg(test)]
#[path = "game/modding/native_level_sound_except_tests.rs"]
mod native_level_sound_except_tests;
#[path = "game/modding/native_level_spawning.rs"]
mod native_level_spawning;
#[path = "game/modding/native_level_ticking.rs"]
mod native_level_ticking;
#[path = "game/modding/native_level_weather.rs"]
mod native_level_weather;
#[path = "game/modding/native_level_world.rs"]
mod native_level_world;
#[path = "game/modding/native_lifecycle.rs"]
mod native_lifecycle;
#[cfg(test)]
#[path = "game/modding/native_live_holder_tests.rs"]
mod native_live_holder_tests;
#[path = "game/modding/native_living_absorption.rs"]
mod native_living_absorption;
#[path = "game/modding/native_living_blocking.rs"]
mod native_living_blocking;
#[path = "game/modding/native_living_body.rs"]
mod native_living_body;
#[path = "game/modding/native_living_effects.rs"]
mod native_living_effects;
#[path = "game/modding/native_living_flags.rs"]
mod native_living_flags;
#[path = "game/modding/native_living_position.rs"]
mod native_living_position;
#[path = "game/modding/native_living_refs.rs"]
mod native_living_refs;
#[cfg(test)]
#[path = "game/modding/native_living_renderers_tests.rs"]
mod native_living_renderers_tests;
#[cfg(test)]
#[path = "game/modding/native_living_travel_sweep_tests.rs"]
mod native_living_travel_sweep_tests;
#[cfg(test)]
#[path = "game/modding/native_living_travel_tests.rs"]
mod native_living_travel_tests;
#[path = "game/modding/native_living_writes.rs"]
mod native_living_writes;
#[path = "game/modding/native_local_player_writes.rs"]
mod native_local_player_writes;
#[path = "game/modding/native_local_sounds.rs"]
mod native_local_sounds;
#[cfg(test)]
#[path = "game/modding/native_makes_piglins_neutral_tests.rs"]
mod native_makes_piglins_neutral_tests;
#[path = "game/modding/native_mapping_presses.rs"]
mod native_mapping_presses;
#[cfg(test)]
#[path = "game/modding/native_menu_inventory_tests.rs"]
mod native_menu_inventory_tests;
#[path = "game/modding/native_menu_places.rs"]
mod native_menu_places;
#[path = "game/modding/native_menu_refs.rs"]
mod native_menu_refs;
#[path = "game/modding/native_menu_slot_kinds.rs"]
mod native_menu_slot_kinds;
#[path = "game/modding/native_menus.rs"]
mod native_menus;
#[path = "game/modding/native_merchant_refs.rs"]
mod native_merchant_refs;
#[path = "game/modding/native_minecart_refs.rs"]
mod native_minecart_refs;
#[path = "game/modding/native_mob_draw.rs"]
mod native_mob_draw;
#[path = "game/modding/native_mob_goals.rs"]
mod native_mob_goals;
#[path = "game/modding/native_mob_navigation.rs"]
mod native_mob_navigation;
#[cfg(test)]
#[path = "game/modding/native_mob_navigation_flags_tests.rs"]
mod native_mob_navigation_flags_tests;
#[path = "game/modding/native_mob_refs.rs"]
mod native_mob_refs;
#[path = "game/modding/native_mob_wanted_position.rs"]
mod native_mob_wanted_position;
#[cfg(test)]
#[path = "game/modding/native_mob_wanted_position_tests.rs"]
mod native_mob_wanted_position_tests;
#[path = "game/modding/native_mobs.rs"]
mod native_mobs;
#[path = "game/modding/native_model_3d.rs"]
mod native_model_3d;
#[path = "game/modding/native_motion.rs"]
mod native_motion;
#[path = "game/modding/native_mounts.rs"]
mod native_mounts;
#[cfg(test)]
#[path = "game/modding/native_move_by_tests.rs"]
mod native_move_by_tests;
#[path = "game/modding/native_namespaced_id.rs"]
mod native_namespaced_id;
#[path = "game/modding/native_net.rs"]
mod native_net;
#[cfg(test)]
#[path = "game/modding/native_on_caught_fire_tests.rs"]
mod native_on_caught_fire_tests;
#[path = "game/modding/native_packet_builders.rs"]
mod native_packet_builders;
#[cfg(test)]
#[path = "game/modding/native_painting_renderers_tests.rs"]
mod native_painting_renderers_tests;
#[path = "game/modding/native_panel.rs"]
mod native_panel;
#[path = "game/modding/native_panel_recipes.rs"]
mod native_panel_recipes;
#[cfg(test)]
#[path = "game/modding/native_panel_wheel_tests.rs"]
mod native_panel_wheel_tests;
#[path = "game/modding/native_part_tables.rs"]
mod native_part_tables;
#[cfg(test)]
#[path = "game/modding/native_part_tables_tests.rs"]
mod native_part_tables_tests;
#[path = "game/modding/native_passage_cross.rs"]
mod native_passage_cross;
#[path = "game/modding/native_passages.rs"]
mod native_passages;
#[cfg(test)]
#[path = "game/modding/native_path_types_tests.rs"]
mod native_path_types_tests;
#[path = "game/modding/native_pending_children.rs"]
mod native_pending_children;
#[path = "game/modding/native_place_feature.rs"]
mod native_place_feature;
#[cfg(test)]
#[path = "game/modding/native_place_feature_tests.rs"]
mod native_place_feature_tests;
#[path = "game/modding/native_placement_modifiers.rs"]
mod native_placement_modifiers;
#[cfg(test)]
#[path = "game/modding/native_placement_state_tests.rs"]
mod native_placement_state_tests;
#[path = "game/modding/native_player_attack.rs"]
mod native_player_attack;
#[path = "game/modding/native_player_break.rs"]
mod native_player_break;
#[path = "game/modding/native_player_cooldowns.rs"]
mod native_player_cooldowns;
#[path = "game/modding/native_player_death.rs"]
mod native_player_death;
#[cfg(test)]
#[path = "game/modding/native_player_destroyed_block_tests.rs"]
mod native_player_destroyed_block_tests;
#[path = "game/modding/native_player_disconnect.rs"]
mod native_player_disconnect;
#[cfg(test)]
#[path = "game/modding/native_player_disconnect_tests.rs"]
mod native_player_disconnect_tests;
#[path = "game/modding/native_player_ender_chest.rs"]
mod native_player_ender_chest;
#[cfg(test)]
#[path = "game/modding/native_player_ender_chest_tests.rs"]
mod native_player_ender_chest_tests;
#[path = "game/modding/native_player_food.rs"]
mod native_player_food;
#[path = "game/modding/native_player_inventory.rs"]
mod native_player_inventory;
#[cfg(test)]
#[path = "game/modding/native_player_inventory_tests.rs"]
mod native_player_inventory_tests;
#[cfg(test)]
#[path = "game/modding/native_player_level_tests.rs"]
mod native_player_level_tests;
#[path = "game/modding/native_player_modes.rs"]
mod native_player_modes;
#[cfg(test)]
#[path = "game/modding/native_player_move_tests.rs"]
mod native_player_move_tests;
#[path = "game/modding/native_player_moves.rs"]
mod native_player_moves;
#[path = "game/modding/native_player_persistent_data.rs"]
mod native_player_persistent_data;
#[cfg(test)]
#[path = "game/modding/native_player_persistent_data_tests.rs"]
mod native_player_persistent_data_tests;
#[path = "game/modding/native_player_place.rs"]
mod native_player_place;
#[cfg(test)]
#[path = "game/modding/native_player_place_tests.rs"]
mod native_player_place_tests;
#[path = "game/modding/native_player_post_effects.rs"]
mod native_player_post_effects;
#[cfg(test)]
#[path = "game/modding/native_player_post_effects_tests.rs"]
mod native_player_post_effects_tests;
#[path = "game/modding/native_player_reach.rs"]
mod native_player_reach;
#[path = "game/modding/native_player_respawn.rs"]
mod native_player_respawn;
#[cfg(test)]
#[path = "game/modding/native_player_respawn_tests.rs"]
mod native_player_respawn_tests;
#[cfg(test)]
#[path = "game/modding/native_player_spectating_tests.rs"]
mod native_player_spectating_tests;
#[path = "game/modding/native_player_stats.rs"]
mod native_player_stats;
#[path = "game/modding/native_player_visibility.rs"]
mod native_player_visibility;
#[cfg(test)]
#[path = "game/modding/native_player_visibility_tests.rs"]
mod native_player_visibility_tests;
#[path = "game/modding/native_pool_elements.rs"]
mod native_pool_elements;
#[path = "game/modding/native_portal_destination.rs"]
mod native_portal_destination;
#[path = "game/modding/native_profiles.rs"]
mod native_profiles;
#[path = "game/modding/native_projectile_launch.rs"]
mod native_projectile_launch;
#[cfg(test)]
#[path = "game/modding/native_projectile_launch_tests.rs"]
mod native_projectile_launch_tests;
#[path = "game/modding/native_projectile_refs.rs"]
mod native_projectile_refs;
#[path = "game/modding/native_projectiles.rs"]
mod native_projectiles;
#[path = "game/modding/native_quick_move.rs"]
mod native_quick_move;
#[path = "game/modding/native_raid_refs.rs"]
mod native_raid_refs;
#[path = "game/modding/native_recipe_properties.rs"]
mod native_recipe_properties;
#[path = "game/modding/native_recipe_sync.rs"]
mod native_recipe_sync;
#[cfg(test)]
#[path = "game/modding/native_recipe_sync_tests.rs"]
mod native_recipe_sync_tests;
#[path = "game/modding/native_recipes.rs"]
mod native_recipes;
#[path = "game/modding/native_registry_entries.rs"]
mod native_registry_entries;
#[cfg(test)]
#[path = "game/modding/native_registry_entries_tests.rs"]
mod native_registry_entries_tests;
#[cfg(test)]
#[path = "game/modding/native_registry_history_tests.rs"]
mod native_registry_history_tests;
#[path = "game/modding/native_registry_ids.rs"]
mod native_registry_ids;
#[cfg(test)]
#[path = "game/modding/native_registry_ids_tests.rs"]
mod native_registry_ids_tests;
#[cfg(test)]
#[path = "game/modding/native_registry_processor_tests.rs"]
mod native_registry_processor_tests;
#[path = "game/modding/native_render_hooks.rs"]
mod native_render_hooks;
#[cfg(test)]
#[path = "game/modding/native_render_hooks_tests.rs"]
mod native_render_hooks_tests;
#[cfg(test)]
#[path = "game/modding/native_render_pair_tests.rs"]
pub(in crate::game) mod native_render_pair_tests;
#[path = "game/modding/native_render_visibility.rs"]
mod native_render_visibility;
#[cfg(test)]
#[path = "game/modding/native_requires_tool_tests.rs"]
mod native_requires_tool_tests;
#[path = "game/modding/native_resources.rs"]
mod native_resources;
#[path = "game/modding/native_ride_player.rs"]
mod native_ride_player;
#[path = "game/modding/native_rotate.rs"]
pub(crate) mod native_rotate;
#[path = "game/modding/native_rule_tests.rs"]
mod native_rule_tests;
#[path = "game/modding/native_saved_chunks.rs"]
mod native_saved_chunks;
#[path = "game/modding/native_scene.rs"]
mod native_scene;
#[path = "game/modding/native_scene_live.rs"]
mod native_scene_live;
#[cfg(test)]
#[path = "game/modding/native_scene_live_tests.rs"]
mod native_scene_live_tests;

#[path = "game/modding/native_client_scoreboard.rs"]
mod native_client_scoreboard;
#[path = "game/modding/native_scoreboard.rs"]
mod native_scoreboard;
#[path = "game/modding/native_scoreboard_teams.rs"]
mod native_scoreboard_teams;

#[path = "game/modding/native_section_dirty.rs"]
mod native_section_dirty;
#[path = "game/modding/native_section_visibility.rs"]
mod native_section_visibility;
#[path = "game/modding/native_server.rs"]
mod native_server;
#[path = "game/modding/native_server_distances.rs"]
mod native_server_distances;
#[path = "game/modding/native_server_max_players.rs"]
mod native_server_max_players;
#[cfg(test)]
#[path = "game/modding/native_server_max_players_tests.rs"]
mod native_server_max_players_tests;
#[path = "game/modding/native_shaders.rs"]
mod native_shaders;
#[cfg(test)]
#[path = "game/modding/native_shield_totem_tests.rs"]
mod native_shield_totem_tests;
#[path = "game/modding/native_singleplayer_owner.rs"]
mod native_singleplayer_owner;
#[cfg(test)]
#[path = "game/modding/native_singleplayer_owner_tests.rs"]
mod native_singleplayer_owner_tests;
#[cfg(test)]
#[path = "game/modding/native_small_reads_tests.rs"]
mod native_small_reads_tests;
#[cfg(test)]
#[path = "game/modding/native_smoke_source_tests.rs"]
mod native_smoke_source_tests;
#[path = "game/modding/native_sound_refs.rs"]
mod native_sound_refs;
#[cfg(test)]
#[path = "game/modding/native_sound_seed_tests.rs"]
mod native_sound_seed_tests;
#[path = "game/modding/native_special_renderers.rs"]
mod native_special_renderers;
#[cfg(test)]
#[path = "game/modding/native_spectator_shader_tests.rs"]
mod native_spectator_shader_tests;
#[path = "game/modding/native_state_shapes.rs"]
mod native_state_shapes;
#[path = "game/modding/native_structure_placement.rs"]
mod native_structure_placement;
#[path = "game/modding/native_structure_processors.rs"]
mod native_structure_processors;
#[path = "game/modding/native_structure_types.rs"]
mod native_structure_types;
#[path = "game/modding/native_structures.rs"]
mod native_structures;
#[cfg(test)]
#[path = "game/modding/native_success_broadcast_tests.rs"]
mod native_success_broadcast_tests;
#[path = "game/modding/native_surface.rs"]
pub(crate) mod native_surface;
#[path = "game/modding/native_tags.rs"]
mod native_tags;
#[path = "game/modding/native_terrain_model.rs"]
mod native_terrain_model;
#[path = "game/modding/native_text_json.rs"]
mod native_text_json;
#[path = "game/modding/native_text_legacy.rs"]
mod native_text_legacy;
#[path = "game/modding/native_text_nbt.rs"]
mod native_text_nbt;
#[path = "game/modding/native_tick_time.rs"]
mod native_tick_time;
#[path = "game/modding/native_ticks.rs"]
mod native_ticks;
#[cfg(test)]
#[path = "game/modding/native_tnt_renderers_tests.rs"]
mod native_tnt_renderers_tests;
#[path = "game/modding/native_trapdoor_ladder.rs"]
pub(crate) mod native_trapdoor_ladder;
#[path = "game/modding/native_tree_grow.rs"]
pub(crate) mod native_tree_grow;
#[path = "game/modding/native_unspawned_mob.rs"]
mod native_unspawned_mob;
#[path = "game/modding/native_viewed_levels.rs"]
mod native_viewed_levels;
#[path = "game/modding/native_visuals.rs"]
mod native_visuals;
#[path = "game/modding/native_world_columns.rs"]
mod native_world_columns;
#[path = "game/modding/native_world_edit.rs"]
mod native_world_edit;
#[path = "game/modding/native_world_read.rs"]
mod native_world_read;
#[path = "game/modding/native_worldgen.rs"]
mod native_worldgen;
#[path = "game/modding/native_worldgen_parts.rs"]
pub(crate) mod native_worldgen_parts;
#[path = "game/mobs/natural_spawning.rs"]
mod natural_spawning;
#[path = "game/mobs/nautili.rs"]
mod nautili;
#[cfg(test)]
#[path = "game/interface/neighbor_notify_tests.rs"]
mod neighbor_notify_tests;
#[path = "game/modding/neoforge_block_maps.rs"]
mod neoforge_block_maps;
#[path = "game/modding/neoforge_mob_maps.rs"]
mod neoforge_mob_maps;
#[path = "game/network/net_harness.rs"]
mod net_harness;
#[path = "game/blocks/nether_portal.rs"]
mod nether_portal;
#[path = "game/runtime/no_resources.rs"]
mod no_resources;
#[path = "game/mobs/ominous_item_spawner/mod.rs"]
mod ominous_item_spawner;
#[path = "game/interface/options.rs"]
mod options;
#[cfg(test)]
#[path = "game/interface/options_close_tests.rs"]
mod options_close_tests;
#[cfg(test)]
#[path = "game/entities/orb_data_tests.rs"]
mod orb_data_tests;
#[cfg(test)]
#[path = "game/modding/orb_merge_tests.rs"]
mod orb_merge_tests;
#[cfg(test)]
#[path = "game/entities/orb_score_store_tests.rs"]
mod orb_score_store_tests;
#[path = "game/world/ordered_tick.rs"]
mod ordered_tick;
#[path = "game/modding/original_player.rs"]
mod original_player;
#[path = "game/world/owner_link.rs"]
mod owner_link;
#[cfg(test)]
#[path = "game/interface/pack_clouds_frame_tests.rs"]
mod pack_clouds_frame_tests;
#[path = "game/modding/pack_finders/mod.rs"]
mod pack_finders;
#[cfg(test)]
#[path = "game/interface/pack_layer_frame_tests.rs"]
mod pack_layer_frame_tests;
#[path = "game/interface/pack_layers.rs"]
mod pack_layers;
#[cfg(test)]
#[path = "game/interface/pack_sources_tests.rs"]
mod pack_sources_tests;
#[path = "game/world/packs/mod.rs"]
mod packs;
#[path = "game/interface/pad/mod.rs"]
mod pad;
#[path = "game/interface/pad_actions.rs"]
mod pad_actions;
#[path = "game/interface/pad_craft.rs"]
mod pad_craft;
#[path = "game/interface/pad_menu.rs"]
mod pad_menu;
#[path = "game/entities/paintings.rs"]
mod paintings;
#[path = "game/mobs/pandas.rs"]
mod pandas;
#[cfg(sunrise_parity_fixtures)]
#[path = "game/parity_fixtures.rs"]
pub mod parity_fixtures;
#[path = "game/modding/part_admission.rs"]
mod part_admission;
#[cfg(test)]
#[path = "game/modding/part_admission_tests.rs"]
mod part_admission_tests;
#[path = "game/mobs/part_shearing.rs"]
mod part_shearing;
#[path = "game/interface/particle_culling.rs"]
mod particle_culling;
#[path = "game/entities/passengers.rs"]
mod passengers;
#[path = "game/modding/payload_transport.rs"]
mod payload_transport;
#[path = "game/mobs/pending_births.rs"]
mod pending_births;
#[path = "game/interface/perf_link.rs"]
mod perf_link;
#[path = "game/interface/perf_remote.rs"]
mod perf_remote;
#[path = "game/world/persist.rs"]
mod persist;
#[path = "game/interface/perspective.rs"]
mod perspective;
#[path = "game/entities/pick_accumulator.rs"]
mod pick_accumulator;
#[path = "game/entities/pick_entity.rs"]
mod pick_entity;
#[path = "game/items/pick_hotbar.rs"]
mod pick_hotbar;
#[path = "game/entities/pickups.rs"]
mod pickups;
#[cfg(test)]
#[path = "game/mobs/piglin_admire_tests.rs"]
mod piglin_admire_tests;
#[cfg(test)]
#[path = "game/mobs/piglin_conversion_tests.rs"]
mod piglin_conversion_tests;
#[path = "game/mobs/piglins.rs"]
mod piglins;
#[path = "game/blocks/piston_carry.rs"]
mod piston_carry;
#[path = "game/blocks/piston_carry_state.rs"]
mod piston_carry_state;
#[path = "game/world/piston_nbt.rs"]
mod piston_nbt;
#[path = "game/blocks/piston_push.rs"]
mod piston_push;
#[cfg(test)]
#[path = "game/blocks/place_feature_tests.rs"]
mod place_feature_tests;
#[path = "game/interface/play_input.rs"]
mod play_input;
#[path = "game/interface/play_keys.rs"]
mod play_keys;
#[cfg(test)]
#[path = "game/interface/play_ping_tests.rs"]
mod play_ping_tests;
#[path = "game/blocks/player_destroy.rs"]
mod player_destroy;
#[cfg(test)]
#[path = "game/items/player_destroy_item_tests.rs"]
mod player_destroy_item_tests;
#[cfg(test)]
#[path = "game/blocks/player_destroy_tests.rs"]
mod player_destroy_tests;
#[path = "game/entities/player_equip.rs"]
mod player_equip;
#[cfg(test)]
#[path = "game/modding/player_equipment_change_tests.rs"]
mod player_equipment_change_tests;
#[path = "game/items/player_hit_sources.rs"]
mod player_hit_sources;
#[cfg(test)]
#[path = "game/network/player_kill_credit_tests.rs"]
mod player_kill_credit_tests;
#[path = "game/entities/player_residency.rs"]
mod player_residency;
#[cfg(test)]
#[path = "game/entities/player_residency_tests.rs"]
mod player_residency_tests;
#[cfg(test)]
#[path = "game/world/player_save_projection_tests.rs"]
mod player_save_projection_tests;
#[path = "game/world/player_spawn.rs"]
mod player_spawn;
#[path = "game/interface/pointer_capture.rs"]
mod pointer_capture;
#[cfg(test)]
#[path = "game/interface/polytone_frame_tests.rs"]
mod polytone_frame_tests;
#[cfg(test)]
#[path = "game/world/portal_ticking_tests.rs"]
mod portal_ticking_tests;
#[path = "game/world/postprocess.rs"]
mod postprocess;
#[path = "game/blocks/potent_sulfur.rs"]
mod potent_sulfur;
#[path = "game/items/potions.rs"]
mod potions;
#[path = "game/blocks/powder_snow.rs"]
mod powder_snow;
#[path = "game/items/primary_items.rs"]
mod primary_items;
#[path = "game/interface/profile_capture.rs"]
mod profile_capture;
#[path = "game/interface/profile_lines.rs"]
mod profile_lines;
#[path = "game/mobs/projectile_block_impacts.rs"]
mod projectile_block_impacts;
#[path = "game/items/projectile_spawned.rs"]
mod projectile_spawned;
#[cfg(test)]
#[path = "game/items/projectile_spawned_tests.rs"]
mod projectile_spawned_tests;
#[path = "game/items/projectile_spawned_validation.rs"]
mod projectile_spawned_validation;
#[cfg(test)]
#[path = "game/modding/published_collision_mob_tests.rs"]
mod published_collision_mob_tests;
#[path = "game/modding/published_loops.rs"]
mod published_loops;
#[cfg(test)]
#[path = "game/modding/published_shape_game_tests.rs"]
mod published_shape_game_tests;
#[path = "game/blocks/pumpkin.rs"]
mod pumpkin;
#[path = "game/entities/pvp.rs"]
mod pvp;
#[path = "game/interface/quick_actions.rs"]
mod quick_actions;
#[path = "game/mobs/raids/mod.rs"]
mod raids;
#[cfg(test)]
#[path = "game/entities/rail_parts_tests.rs"]
mod rail_parts_tests;
#[cfg(test)]
#[path = "game/blocks/random_order_tests.rs"]
mod random_order_tests;
#[path = "game/interface/random_textures.rs"]
mod random_textures;
#[path = "game/network/raw_backlog.rs"]
mod raw_backlog;
#[path = "game/items/recipe_book.rs"]
mod recipe_book;
#[cfg(test)]
#[path = "game/interface/recipe_book_variants_tests.rs"]
mod recipe_book_variants_tests;
#[cfg(test)]
#[path = "game/interface/recipe_properties_tests.rs"]
mod recipe_properties_tests;
#[cfg(test)]
#[path = "game/modding/rectangular_recipe_remainder_tests.rs"]
mod rectangular_recipe_remainder_tests;
#[path = "game/modding/registration_capability_coverage.rs"]
mod registration_capability_coverage;
#[path = "game/network/remote/mod.rs"]
mod remote;
#[path = "game/network/remote_block_writes.rs"]
mod remote_block_writes;
#[path = "game/network/remote_camera.rs"]
mod remote_camera;
#[path = "game/network/remote_chat.rs"]
mod remote_chat;
#[path = "game/network/remote_chunks.rs"]
mod remote_chunks;
#[path = "game/network/remote_mob_data.rs"]
mod remote_mob_data;
#[path = "game/network/remote_mob_particles.rs"]
mod remote_mob_particles;
#[path = "game/network/remote_mob_use.rs"]
mod remote_mob_use;
#[cfg(test)]
#[path = "game/network/remote_mob_use_tests.rs"]
mod remote_mob_use_tests;
#[path = "game/network/remote_objects.rs"]
mod remote_objects;
#[path = "game/network/remote_projectiles.rs"]
mod remote_projectiles;
#[path = "game/network/remote_removal.rs"]
mod remote_removal;
#[cfg(test)]
#[path = "game/world/remote_save_tests.rs"]
mod remote_save_tests;
#[path = "game/network/remote_stations.rs"]
mod remote_stations;
#[path = "game/network/remote_tags.rs"]
mod remote_tags;
#[path = "game/network/remote_vehicles.rs"]
mod remote_vehicles;
#[path = "game/interface/render_control.rs"]
mod render_control;
#[path = "game/interface/render_settings.rs"]
mod render_settings;
#[path = "game/interface/render_tick.rs"]
mod render_tick;
#[path = "game/runtime/replay.rs"]
mod replay;
#[path = "game/runtime/repro.rs"]
mod repro;
#[path = "game/entities/residency_tick.rs"]
mod residency_tick;
#[path = "game/entities/resident_state.rs"]
mod resident_state;
#[path = "game/runtime/resource_load.rs"]
mod resource_load;
#[path = "game/runtime/resource_loader.rs"]
mod resource_loader;
#[path = "game/runtime/resource_tables.rs"]
mod resource_tables;
#[path = "game/blocks/respawn_anchor.rs"]
mod respawn_anchor;
#[path = "game/world/respawn_position.rs"]
mod respawn_position;
#[cfg(test)]
#[path = "game/blocks/respawn_position_tests.rs"]
mod respawn_position_tests;
#[cfg(test)]
#[path = "game/modding/rule_replacements.rs"]
mod rule_replacements;
#[path = "game/world/runtime_levels.rs"]
mod runtime_levels;
#[cfg(test)]
#[path = "game/world/runtime_levels_tests.rs"]
mod runtime_levels_tests;
#[cfg(test)]
#[path = "game/runtime/runtime_reset_tests.rs"]
mod runtime_reset_tests;
#[cfg(test)]
#[path = "game/world/save_format_tests.rs"]
mod save_format_tests;
#[cfg(test)]
#[path = "game/world/save_load_pass_tests.rs"]
mod save_load_pass_tests;
#[cfg(test)]
#[path = "game/world/save_pass_tests.rs"]
mod save_pass_tests;
#[path = "game/runtime/saves_control.rs"]
mod saves_control;
#[path = "game/world/saving.rs"]
mod saving;
#[path = "game/modding/scheduled_tasks.rs"]
mod scheduled_tasks;
#[path = "game/interface/scoreboard/mod.rs"]
mod scoreboard;
#[path = "game/interface/screen_backgrounds.rs"]
mod screen_backgrounds;
#[path = "game/interface/screen_build.rs"]
mod screen_build;
#[path = "game/interface/screen_build_fixture.rs"]
mod screen_build_fixture;
#[path = "game/interface/screen_info.rs"]
mod screen_info;
#[cfg(test)]
#[path = "game/interface/screen_render_phase_tests.rs"]
mod screen_render_phase_tests;
#[path = "game/interface/screen_room.rs"]
mod screen_room;
#[path = "game/interface/screen_widgets.rs"]
mod screen_widgets;
#[path = "game/interface/screens.rs"]
mod screens;
#[path = "game/runtime/screenshot_sequence.rs"]
mod screenshot_sequence;
#[path = "game/interface/screenshot_writer.rs"]
mod screenshot_writer;
#[path = "game/blocks/sculk.rs"]
mod sculk;
#[path = "game/blocks/sculk_fixture.rs"]
mod sculk_fixture;
#[path = "game/entities/seats.rs"]
mod seats;
#[path = "game/world/server_lifecycle/mod.rs"]
mod server_lifecycle;
#[path = "game/network/server_links.rs"]
mod server_links;
#[path = "game/world/server_packs.rs"]
mod server_packs;
#[path = "game/world/server_reload/mod.rs"]
mod server_reload;
#[path = "game/world/server_travel.rs"]
mod server_travel;
#[cfg(test)]
#[path = "game/administration/setblock_order_tests.rs"]
mod setblock_order_tests;
#[cfg(test)]
#[path = "game/interface/shader_pack_switch_tests.rs"]
mod shader_pack_switch_tests;
#[path = "game/interface/shader_packs.rs"]
mod shader_packs;
#[cfg(test)]
#[path = "game/mobs/shear_refind_tests.rs"]
mod shear_refind_tests;
#[path = "game/mobs/sheep_dye.rs"]
mod sheep_dye;
#[path = "game/blocks/shelf.rs"]
mod shelf;
#[path = "game/items/shield.rs"]
mod shield;
#[path = "game/mobs/shoulders.rs"]
mod shoulders;
#[path = "game/interface/sign_edit.rs"]
mod sign_edit;
#[path = "game/modding/slot_changes.rs"]
mod slot_changes;
#[cfg(test)]
#[path = "game/modding/slot_pickup_tests.rs"]
mod slot_pickup_tests;
#[path = "game/modding/slot_rules.rs"]
mod slot_rules;
#[cfg(test)]
#[path = "game/modding/slot_rules_tests.rs"]
mod slot_rules_tests;
#[cfg(test)]
#[path = "game/modding/slot_take_tests.rs"]
mod slot_take_tests;
#[path = "game/modding/slot_takes.rs"]
mod slot_takes;
#[path = "game/interface/smithing_icons.rs"]
mod smithing_icons;
#[path = "game/mobs/sniffers.rs"]
mod sniffers;
#[path = "game/network/social.rs"]
mod social;
#[cfg(test)]
#[path = "game/world/spawn_save_tests.rs"]
mod spawn_save_tests;
#[cfg(test)]
#[path = "game/mobs/spawn_yaw_tests.rs"]
mod spawn_yaw_tests;
#[path = "game/mobs/spawner.rs"]
mod spawner;
#[cfg(test)]
#[path = "game/mobs/spawner_client_tick_tests.rs"]
pub(in crate::game) mod spawner_client_tick_tests;
#[path = "game/items/spear.rs"]
mod spear;
#[path = "game/items/spear_players.rs"]
mod spear_players;
#[path = "game/items/special_attack_fixture.rs"]
mod special_attack_fixture;
#[path = "game/items/special_attacks.rs"]
mod special_attacks;
#[path = "game/interface/spectator.rs"]
mod spectator;
#[path = "game/blocks/spill.rs"]
mod spill;
#[path = "game/blocks/splash_water.rs"]
mod splash_water;
#[path = "game/items/stack_attribute_tooltip.rs"]
mod stack_attribute_tooltip;
#[path = "game/modding/stacked_on_other.rs"]
mod stacked_on_other;
#[cfg(test)]
#[path = "game/modding/stacked_on_other_tests.rs"]
mod stacked_on_other_tests;
#[cfg(test)]
#[path = "game/entities/stand_break_loot_tests.rs"]
mod stand_break_loot_tests;
#[cfg(test)]
#[path = "game/entities/stand_command_hurt_tests.rs"]
mod stand_command_hurt_tests;
#[cfg(test)]
#[path = "game/interface/stand_equipment_change_tests.rs"]
mod stand_equipment_change_tests;
#[cfg(test)]
#[path = "game/entities/stand_low_health_tests.rs"]
mod stand_low_health_tests;
#[cfg(test)]
#[path = "game/modding/start_riding_refusal_tests.rs"]
mod start_riding_refusal_tests;
#[cfg(test)]
#[path = "game/runtime/startup_cpu_tests.rs"]
mod startup_cpu_tests;
#[path = "game/interface/startup_menu.rs"]
mod startup_menu;
#[path = "game/interface/startup_options.rs"]
mod startup_options;
#[path = "game/runtime/startup_progress.rs"]
pub(crate) mod startup_progress;
#[cfg(test)]
#[path = "game/interface/state_at_viewpoint_tests.rs"]
mod state_at_viewpoint_tests;
#[cfg(feature = "state-digest")]
#[path = "game/world/state_digest.rs"]
mod state_digest;
#[path = "game/modding/state_resolver.rs"]
mod state_resolver;
#[cfg(test)]
#[path = "game/runtime/stateful_block_entity_tests.rs"]
mod stateful_block_entity_tests;
#[path = "game/runtime/station_control.rs"]
mod station_control;
#[path = "game/blocks/stations.rs"]
mod stations;
#[path = "game/interface/stats.rs"]
mod stats;
#[path = "game/interface/stats_screen.rs"]
mod stats_screen;
#[path = "game/entities/stock_layer_mask.rs"]
mod stock_layer_mask;
#[cfg(test)]
#[path = "game/items/stop_using_tests.rs"]
mod stop_using_tests;
#[path = "game/world/storage.rs"]
mod storage;
#[cfg(test)]
#[path = "game/runtime/storage_alias_tests.rs"]
mod storage_alias_tests;
#[path = "game/administration/structure_blocks.rs"]
mod structure_blocks;
#[path = "game/administration/structure_edit.rs"]
mod structure_edit;
#[path = "game/administration/structures.rs"]
mod structures;
#[path = "game/interface/studio.rs"]
mod studio;
#[path = "game/interface/subtitle_hud.rs"]
mod subtitle_hud;
#[path = "game/interface/subtitles.rs"]
mod subtitles;
#[cfg(test)]
#[path = "game/blocks/sulfur_bucket_tests.rs"]
mod sulfur_bucket_tests;
#[path = "game/mobs/sulfur_cubes.rs"]
mod sulfur_cubes;
#[cfg(test)]
#[path = "game/mobs/sulfur_prime_phase_tests.rs"]
mod sulfur_prime_phase_tests;
#[path = "game/network/swing_packet.rs"]
mod swing_packet;
#[cfg(test)]
#[path = "game/interface/sync_background_tests.rs"]
mod sync_background_tests;
#[path = "game/items/table_enchant.rs"]
mod table_enchant;
#[path = "game/administration/test_block_edit.rs"]
mod test_block_edit;
#[path = "game/administration/test_blocks.rs"]
mod test_blocks;
#[path = "game/administration/test_entities.rs"]
mod test_entities;
#[path = "game/administration/test_environments.rs"]
mod test_environments;
#[path = "game/administration/test_instance_edit.rs"]
mod test_instance_edit;
#[path = "game/administration/test_instances.rs"]
mod test_instances;
#[cfg(test)]
#[path = "game/runtime/test_pos_guest_tests.rs"]
mod test_pos_guest_tests;
#[path = "game/entities/text_displays.rs"]
mod text_displays;
#[cfg(test)]
#[path = "game/interface/text_expression_tests.rs"]
mod text_expression_tests;
#[path = "game/mobs/thrown_item_draw.rs"]
mod thrown_item_draw;
#[path = "game/world/tick_script.rs"]
mod tick_script;
#[path = "game/interface/titles.rs"]
mod titles;
#[path = "game/entities/tnt.rs"]
mod tnt;
#[path = "game/interface/toast_manager.rs"]
mod toast_manager;
#[path = "game/blocks/tool_modification.rs"]
mod tool_modification;
#[path = "game/items/totem.rs"]
mod totem;
#[path = "game/items/trading.rs"]
mod trading;
#[cfg(test)]
#[path = "game/runtime/transaction_entities_tests.rs"]
mod transaction_entities_tests;
#[path = "game/runtime/transactions.rs"]
mod transactions;
#[path = "game/blocks/transient_blocks.rs"]
mod transient_blocks;
#[path = "game/mobs/trial_spawner.rs"]
mod trial_spawner;
#[path = "game/blocks/trial_visuals.rs"]
mod trial_visuals;
#[path = "game/items/trident.rs"]
mod trident;
#[path = "game/interface/tutorial.rs"]
mod tutorial;
#[path = "game/modding/typed_payloads.rs"]
mod typed_payloads;
#[path = "game/runtime/ui_control.rs"]
mod ui_control;
#[cfg(test)]
#[path = "game/runtime/ui_core_shader_tests.rs"]
mod ui_core_shader_tests;
#[path = "game/interface/upload_pace.rs"]
mod upload_pace;
#[path = "game/items/use_hand.rs"]
mod use_hand;
#[cfg(test)]
#[path = "game/items/use_same_item_tests.rs"]
mod use_same_item_tests;
#[cfg(test)]
#[path = "game/items/use_stop_tests.rs"]
mod use_stop_tests;
#[path = "game/items/using_item.rs"]
mod using_item;
#[path = "game/blocks/vault.rs"]
mod vault;
#[cfg(test)]
#[path = "game/entities/vehicle_removal_tests.rs"]
pub(in crate::game) mod vehicle_removal_tests;
#[path = "game/entities/vehicle_teleport.rs"]
mod vehicle_teleport;
#[cfg(test)]
#[path = "game/blocks/vibration_source_tests.rs"]
mod vibration_source_tests;
#[cfg(test)]
#[path = "game/blocks/vibration_tests.rs"]
mod vibration_tests;
#[path = "game/world/view_pass.rs"]
mod view_pass;
#[path = "game/network/viewed_worlds/mod.rs"]
mod viewed_worlds;
#[path = "game/network/villager_client.rs"]
mod villager_client;
#[path = "game/world/villager_professions.rs"]
mod villager_professions;
#[path = "game/mobs/villagers.rs"]
mod villagers;
#[cfg(test)]
#[path = "game/world/visibility_unwind_tests.rs"]
mod visibility_unwind_tests;
#[path = "game/world/wake_time.rs"]
mod wake_time;
#[path = "game/mobs/wandering_trader.rs"]
mod wandering_trader;
#[path = "game/blocks/waxing.rs"]
mod waxing;
#[cfg(test)]
#[path = "game/blocks/waxing_tests.rs"]
mod waxing_tests;
#[path = "game/world/weather.rs"]
mod weather;
#[path = "game/interface/win.rs"]
mod win;
#[path = "game/interface/wind.rs"]
mod wind;
#[path = "game/runtime/window_handler.rs"]
mod window_handler;
#[path = "game/runtime/window_title.rs"]
mod window_title;
#[path = "game/mobs/wither.rs"]
mod wither;
#[cfg(test)]
#[path = "game/mobs/wither_star_tests.rs"]
mod wither_star_tests;
#[path = "game/world/world_border.rs"]
mod world_border;
#[path = "game/world/world_entry.rs"]
mod world_entry;
#[cfg(test)]
#[path = "game/interface/world_fluid_tests.rs"]
mod world_fluid_tests;
#[path = "game/blocks/world_fluids.rs"]
mod world_fluids;
#[cfg(test)]
#[path = "game/blocks/world_fluids_tests.rs"]
mod world_fluids_tests;
#[path = "game/world/world_loading.rs"]
mod world_loading;
#[cfg(test)]
#[path = "game/modding/world_name_tag_tests.rs"]
mod world_name_tag_tests;
#[path = "game/world/world_rest.rs"]
mod world_rest;
#[path = "game/world/world_session.rs"]
mod world_session;
#[cfg(test)]
#[path = "game/blocks/world_test_def_tests.rs"]
mod world_test_def_tests;
#[path = "game/world/world_update.rs"]
mod world_update;
#[path = "game/items/worn_boots.rs"]
mod worn_boots;
#[path = "game/mobs/xp_orb_targeting.rs"]
mod xp_orb_targeting;
#[cfg(test)]
#[path = "game/items/xp_repair_ratio_tests.rs"]
mod xp_repair_ratio_tests;
#[path = "game/mobs/zombie_reinforcements.rs"]
mod zombie_reinforcements;
#[path = "game/mobs/zombie_villagers.rs"]
mod zombie_villagers;
pub use hot_paths::{HotPathsMods, hot_paths_next_block_entity, note_hot_paths_listener_call};
use sunrise_client_gameplay::frame_autotune;
use sunrise_client_gameplay::occlusion_culling;
use sunrise_gameplay::lan::idle as lan_idle;
use sunrise_gameplay::{interaction_shapes, item_areas, mob_index, tick_profile};

use event_loop::EventLoopCtl;
use hand::interact_location;
pub(crate) use hit::Hit;
use loading_window::{
    build_renderer, loading_frame, loading_renderer, prepare_renderer_textures, start_game,
};
use resource_loader::{
    EntityModels, GameCpuResources, Loaded, LoaderMsg, PendingWorldEntry, WorldRestLoader,
    run_loading, write_report,
};
use sunrise_client::random_source;
use sunrise_client_render::block_entity_reach::Reach;
// `Action` here is the title screen's buttons (`title::Action`).
use self::api_values::TryIntoApi;
pub(crate) use api_values::{block_jump_factor, block_speed_factor, is_ladder};
pub(crate) use launch_options::Options;
pub use native_block_entity_draw::BlockEntityDraw;
pub use native_data_maps::DataMapType;
pub use native_hooks::{PhantomSpawnResult, SleepProblem};
pub use native_mob_draw::{MobRender, PlayerRender};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
        mpsc::{Receiver, TryRecvError},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use sunrise_client::actions::Action as Mapping;
use sunrise_client::held::Source;
use sunrise_client::spans::{startup, time as span};
use sunrise_client::{
    Result,
    assets::{Assets, RenderAssets},
    audio::Audio,
    biome::Biomes,
    block_entity_render::{
        self, BlockEntityRenderer, CacheOptions, Scene as BlockEntityScene, StaticCacheFrameStats,
        StaticSectionSnapshot,
    },
    blocks::{BlockFacts, Fluid},
    clock::FixedStep,
    collisions,
    containers::{BlockEntity, Providers},
    crafting::Recipes,
    creative::{Creative, Names},
    entities::{Experience, ExperienceOrb, ItemEntity},
    environment::Environment,
    first_person::{Hand, SwingAnimation},
    furnace::FurnaceCooking,
    gfx::{Frame, Renderer},
    graphics::EntityCullingMode,
    hud::{HudAtlas, HudBatch, rgba},
    hud_layout, hud_overlays,
    items::{ItemStack, Items},
    keymapping::{KeyAction, SNEAK, SPRINT},
    light::LightTables,
    mesher::{MeshTables, Vertex},
    mobs::{Mob, MobType, MobTypes},
    models,
    motion::{MotionResources, PlayerMotion},
    mouse::{LookOptions, MouseLook},
    particles::Particle,
    save::{Level, Persistent, SaveDir},
    settings::Settings,
    spans,
    sprint::Keys,
    streaming::{Streaming, WorkerContext},
    testgen::TestTerrain,
    title::{Action, Title},
    vanillagen::VanillaTerrain,
    world::World,
};
use sunrise_core::{
    biome::{BiomeZoom, obfuscate_seed},
    geometry::{Aabb, Direction},
    position::BlockPos,
};
use sunrise_core::{geometry::Vec3, random::RandomSource};
use sunrise_mod_package::selection::{Argument, LaunchArguments};
use winit::{
    application::ApplicationHandler,
    error::EventLoopError,
    event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};
use {
    sunrise_core::dimension::Dimension as LevelDimension,
    sunrise_world::{
        block_collision::BlockCollisionRegistry,
        collision_context::{CollisionContext, CollisionEntity, CollisionItem},
        entity_move::MoverType,
        entity_obstruction::{EntityIdentity, EntityObstructionWorld, ObstructionEntity},
        fluid::FluidState,
        level::{
            Behaviours, BucketContent, EntityQuery, Landing, LevelAccess, LevelEvent, PlaceContext,
            PlacementContext, PlayerContext, PlayerEffect, RandomTickMaskUpdate, Ticks,
            flags as level_flags,
        },
        registry::BlockStateId,
    },
};

/// A frame's finished meshes up to this many upload at once, unqueued.
const AUTO_UPLOADS_AT_ONCE: usize = 8;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    Playing,
    GameModeSwitcher,
    Paused,
    Options,
    Chat,
    Creative,
    Inventory,
    Dead,
    /// The Statistics screen.
    Stats,
    /// The Advancements screen.
    Advancements,
    /// WorldOptionsScreen (game/network/lan_screen.rs).
    WorldOptions,
    /// Play with Friends, from the pause menu's Friends button
    /// (`friends_screen`, game/network/friends.rs).
    Friends,
    /// `SocialInteractionsScreen` (`social_screen`, game/network/social.rs).
    Social,
    /// CommandBlockEditScreen (game/administration/command_blocks.rs).
    CommandBlock,
    /// A game master block's edit screen (game/interface/block_editor.rs).
    BlockEditor,
    /// ServerLinksDialogScreen: a server's links (game/network/server_links.rs).
    ServerLinks,
    /// ConfirmLinkScreen for a server link (game/network/server_links.rs).
    ConfirmLink,
    /// Parentless PopupScreen opened by the server's demo event.
    DemoIntro,
    /// DialogScreen.WarningScreen: the note about custom screens.
    DialogWarning,
    /// WinScreen: the end poem and the credits (game/interface/win.rs).
    Win,
    /// DebugOptionsScreen, F3+F6 (game/interface/debug_options.rs).
    DebugOptions,
    /// A screen of a native mod's own (`mod_screen_open`): it draws and
    /// takes its input through the screen hooks.
    Mod,
    /// PvP & HUD's widget editor (game/interface/hud_editor.rs).
    HudEditor,
}
impl Screen {
    /// The id native mods see for this screen (`ScreenView.id`, the
    /// `screen_render_post` hook): one per variant, and a hook's contract, so
    /// it stays the same from release to release. A new variant must add one.
    fn id(self) -> &'static str {
        match self {
            Screen::Playing => "playing",
            Screen::GameModeSwitcher => "game_mode_switcher",
            Screen::Paused => "paused",
            Screen::Options => "options",
            Screen::Chat => "chat",
            Screen::Creative => "creative",
            Screen::Inventory => "inventory",
            Screen::Dead => "dead",
            Screen::Stats => "stats",
            Screen::Advancements => "advancements",
            Screen::WorldOptions => "world_options",
            Screen::Friends => "friends",
            Screen::Social => "social",
            Screen::CommandBlock => "command_block",
            Screen::BlockEditor => "block_editor",
            Screen::ServerLinks => "server_links",
            Screen::ConfirmLink => "confirm_link",
            Screen::DemoIntro => "demo_intro",
            Screen::DialogWarning => "dialog_warning",
            Screen::Win => "win",
            Screen::DebugOptions => "debug_options",
            Screen::Mod => "mod",
            Screen::HudEditor => "hud_editor",
        }
    }
}
/// The vanilla `state` a `transformer` table entry offers to `ability`: hoe and
/// shovel tables have one action each; the axe's strip, scrape and wax-off lists
/// are one untagged list in `BlockTransformers.bootstrap`, so its particle names it.
fn vanilla_tool_state(
    transformer: &str,
    particle: &str,
    ability: mod_api::ToolAbility,
    state: BlockStateId,
) -> Option<BlockStateId> {
    use mod_api::ToolAbility::*;
    let action = match transformer {
        "minecraft:axe" => match particle {
            "scrape" => AxeScrape,
            "wax_off" => AxeWaxOff,
            _ => AxeStrip,
        },
        "minecraft:shovel" => ShovelFlatten,
        "minecraft:hoe" => HoeTill,
        _ => return None,
    };
    (action == ability).then_some(state)
}

struct Mining {
    pos: [i32; 3],
    progress: f32,
    /// The latest destroy-speed query, reused for server crack feedback.
    per_tick: f32,
    ticks: u32,
    /// The stage's display-only answer, dropped with this mining state.
    crack: Option<client_cracks::ClientCrack>,
}
impl Mining {
    fn stage(&self) -> Option<u8> {
        (self.progress > 0.0).then(|| ((self.progress * 10.0) as u8).min(9))
    }
}
/// What a transform does beyond the swap: its `transform_type` and the `drop_strategy` its loot
/// table's stacks pop with (both empty for a tool ability outside the table).
#[derive(Clone, Debug, Default)]
pub(crate) struct TransformKind {
    pub(crate) transform_type: String,
    pub(crate) drop_strategy: String,
}
#[derive(Clone, Copy)]
struct TransformContext<'a> {
    world: &'a World,
    player: Option<u32>,
    hand: mod_api::Hand,
    blocking: bool,
    simulated: bool,
}

/// What a falling block's tick did (`FallingBlockEntity.tick`).
enum FallStep {
    Falling,
    Gone,
    /// Gave up falling: drops itself as an item here.
    Dropped(Vec3),
    /// Hit the ground in this block.
    Landed([i32; 3]),
}
/// A guest's right click comes as one packet per hand and part
/// (`Minecraft.startUseItem`): `ServerGamePacketListenerImpl.handleUseItemOn`
/// does the block's and `ItemStack.useOn`'s part, `handleUseItem` the
/// item's own (`Item.use`). Running both for either would use the item twice.
#[derive(Clone, Copy)]
enum GuestUse {
    OnBlock {
        hand: Hand,
        block_fell_through: bool,
    },
    Item(Hand),
    /// `handleInteract`: `Player.interactOn` with the hand, nothing else (the
    /// client sends UseItem itself when the entity passed), on the entity the
    /// packet named: `ServerGamePacketListenerImpl.handleInteract` resolves
    /// `packet.entityId()` once and range checks it, so the click never picks
    /// an entity again from the view.
    Entity(Hand, InteractTarget),
}
impl GuestUse {
    fn on_block(hand: Hand) -> Self {
        Self::OnBlock {
            hand,
            block_fell_through: false,
        }
    }
}
/// The entity an interact packet names and where its client clicked it (the
/// packet's location, measured from the entity's position): what
/// `ServerGamePacketListenerImpl.handleInteract` takes from
/// `ServerboundInteractPacket` and hands to `Player.interactOn`, which passes
/// the point on to `interactAt`.
#[derive(Clone, Copy)]
struct InteractTarget {
    id: i32,
    location: [f64; 3],
}

/// The client world as seen by the vanilla block-behaviour library.
struct GameLevel<'w> {
    cake_probe: bool,
    cake_requested: std::cell::Cell<bool>,
    cake_answer: Option<bool>,
    api_level: Option<mod_api::LevelRef>,
    part_state_index: Option<
        &'w std::sync::Arc<
            std::sync::Mutex<
                sunrise_mod_runtime::attachments::index::AttachmentIndex<mod_api::AttachmentHolder>,
            >,
        >,
    >,
    attachment_query_scopes: &'w sunrise_mod_runtime::attachments::AttachmentQueryScopes,
    positions: piston_carry_state::PositionState<'w>,
    border: &'w sunrise_core::geometry::border::BorderGeometry,
    world: &'w mut World,
    command_block_transition: Option<command_blocks::CommandBlockTransition>,
    redstone_experiments: bool,
    facts: &'w BlockFacts,
    /// The block-state facts a part reads through `LevelHost::block_states`.
    tables: native_block_tables::BlockTables<'w>,
    collision: &'w BlockCollisionRegistry<'w>,
    events: &'w mut Vec<LevelEvent>,
    dimension: LevelDimension,
    darken: i32,
    /// The environment of the level's dimension (`SUN_ANGLE`).
    environment: &'w Environment,
    raining: bool,
    thundering: bool,
    /// The world's enabled feature flags as this side knows them (`LevelReader.enabledFeatures`).
    enabled_features: &'w std::sync::Arc<[String]>,
    player_box: Option<([f64; 3], [f64; 3])>,
    /// The LAN guests in this level who trigger blocks (not spectators).
    guest_boxes: Vec<([f64; 3], [f64; 3])>,
    /// Other players that block placement during a guest turn, including live
    /// ancestor bodies; None outside one, where `guest_boxes` are those players.
    building_player_boxes: Option<Vec<([f64; 3], [f64; 3])>>,
    player_collision: Option<PlayerCollision>,
    /// Minecart boxes (detector rails).
    cart_boxes: Vec<([f64; 3], [f64; 3])>,
    /// The item frames a comparator reads through a conductor.
    item_frames: &'w [sunrise_client::entities::ItemFrame],
    /// The minecarts, in `cart_boxes` order, for a detector rail's comparator.
    carts: &'w mut sunrise_client::join_seq::JoinList<minecarts::Minecart, false>,
    /// Arrow and thrown trident boxes (wooden buttons).
    arrow_boxes: Vec<([f64; 3], [f64; 3])>,
    /// Every entity on the walk; see `entity::for_each_entity`.
    other_entities: entity::EntityLists<'w>,
    /// End crystals, which aren't on the walk yet.
    crystals: &'w [dragon::Crystal],
    /// The fishing hooks here, which aren't on the walk either. None ignores
    /// block triggers, so each counts on a plate or tripwire. The flag says
    /// whether it also reaches `entityInside` (`applyEffectsFromBlocks`): not
    /// a hook on an entity, whose tick skips that step.
    trigger_hooks: Vec<(Aabb, bool)>,
    /// The lightning bolts here: none ignores block triggers
    /// (`Entity.isIgnoringBlockTriggers`).
    trigger_extras: Vec<Aabb>,
    biomes: &'w Biomes,
    biome_keys: Option<&'w mod_api::__private::BiomeKeys>,
    /// `BiomeManager`'s zoom over this level's seed, for `biome_name`.
    biome_zoom: sunrise_core::biome::BiomeZoom,
    seed: i64,
    behaviours: &'w Behaviours,
    items: &'w Items,
    /// Cooking recipes' experience, which a removed furnace pays out.
    recipes: &'w sunrise_client::crafting::Recipes,
    /// Container loot tables (`unpackLootTable`).
    loot: Option<std::sync::Arc<sunrise_world::loot::LootData>>,
    loot_storage: std::sync::Arc<sunrise_world::loot::copy::StorageData>,
    /// The level's world clocks, for `time_check` in loot conditions.
    loot_clocks: std::collections::BTreeMap<String, i64>,
    registry: &'static sunrise_world::registry::BlockRegistry,
    block_registries: &'w sunrise_client::command::args::Registries,
    rules: sunrise_world::level::GameRules,
    /// `Difficulty.getId()` (peaceful 0 .. hard 3).
    difficulty: i32,
    /// The feet of the players who keep fire spreading around them: the local
    /// player and the LAN guests in this level, spectators aside.
    fire_players: Vec<[f64; 3]>,
    /// Block loot random sequences (`ServerLevel.getRandomSequence`).
    rand_sequences: &'w mut sunrise_client::random_sequences::RandomSequences,
    pending: &'w mut spill::Pending,
    spill: &'w mut spill::Spill,
    /// Where the chunk tickets let blocks tick.
    tickets: sunrise_client::ticket_levels::Levels,
    random_tick_sections: &'w mut sunrise_client::random_tick_sections::RandomTickSections,
    /// Players with each chest open (a trapped chest's signal), also what a
    /// recount of the openers replaces (`ContainerOpenersCounter.recheckOpeners`).
    lid_openers: &'w mut HashMap<(i32, i32, i32), i32>,
    /// The containers players have open at this moment, by position: the local
    /// player's and the guests' (`getEntitiesWithContainerOpen`). A double
    /// chest is in it as both halves.
    open_containers: &'w [(i32, i32, i32)],
    /// The chests copper golems have open, a double chest as both halves. They
    /// keep their own count (`chest_openers`), so they stay out of `open_containers`.
    golem_chests: &'w [(i32, i32, i32)],
    /// `Level.isClientSide`: a client's copy of the level (a guest of a LAN
    /// world, a window on its server thread, a dedicated server's client).
    client_side: bool,
    /// Only the integrated or dedicated server dispatches native observations.
    native_authoritative: bool,
    native_player: Option<u128>,
    /// The acting player's entity id, beside `native_player`.
    native_player_entity: Option<i32>,
    /// Direct liquid part writes may not bypass an open Game transaction.
    liquid_transaction_open: bool,
    /// `Level.isDebug`: `setBlock` changes nothing.
    debug: bool,
    native_sounds: &'w mut Vec<creatures::NativeSound>,
    loop_context: &'w sunrise_world::native_load_context::LoopContext,
    /// This side's level data, for the read-only level a block's sound type is asked in.
    side: native_surface::GameSide<'w>,
    native_particles: &'w mut Vec<lan_particles::QueuedParticle>,
    sound_seeds: Option<&'w sunrise_client::lan::hub::Hub>,
    drop_entity: Option<mod_api::EntityRef>,
    /// `LootContextParams.INTERACTING_ENTITY` of a block-interaction drop: the
    /// player whose click `use_without_item` runs for.
    interacting: Option<sunrise_world::loot::LootEntity>,
    /// `LootContextParams.TOOL` of a block-interaction drop, when an item
    /// triggered the interaction (`None` for bare-hand `use_without_item`).
    interacting_tool: Option<ItemStack>,
    /// What a location predicate of such a drop reads beyond the level's own
    /// fields (`blocks::location_world`).
    location: location_world::LocationRefs<'w>,
    /// The block a mod's machine breaks and the tool it breaks it with
    /// (`native_machine_destroy_block`): `Block.dropResources` rolls that
    /// block's loot with the tool instead of with none. Neighbours that pop
    /// off because of the break drop as with no tool, as in vanilla.
    drop_tool: Option<((i32, i32, i32), Option<ItemStack>)>,
    /// Only this destroyed block has an entity as its loot `this` parameter.
    /// Neighbour/default/machine drops keep the no-entity context.
    drop_breaker_at: Option<BlockPos>,
    /// Where `Block.dropResources` puts the drops instead of popping them,
    /// while a mod's machine breaks a block (`native_machine_destroy_block`).
    drop_into: Option<Vec<ItemStack>>,
}
struct PlayerCollision {
    y: f64,
    descending: bool,
    fall_distance: f64,
    main_hand_item: Arc<str>,
    main_hand_count: i32,
    leather_boots: bool,
}
/// The boxes of the entities that block building, as obstruction candidates.
struct BuildingObstruction(Vec<Aabb>);
impl EntityObstructionWorld for BuildingObstruction {
    type Error = std::convert::Infallible;
    fn obstruction_candidates(
        &mut self,
        _excluded: Option<EntityIdentity>,
        bounds: Aabb,
    ) -> std::result::Result<Vec<ObstructionEntity>, Self::Error> {
        Ok(self
            .0
            .iter()
            .filter(|b| b.intersects(bounds))
            .enumerate()
            .map(|(i, &bounds)| ObstructionEntity {
                identity: EntityIdentity(i as u64),
                root_vehicle: EntityIdentity(i as u64),
                bounds,
                removed: false,
                spectator: false,
                blocks_building: true,
            })
            .collect())
    }
}
/// Whether a block's successful use is `InteractionResult.SUCCESS_SERVER`,
/// which a client does not predict: `useWithoutItem` of a bed, a light
/// block, a charged respawn anchor or a sign. Every other block use the
/// level answers with SUCCESS is `SUCCESS` (`SwingSource.PREDICTED`).
fn server_only_block_use(
    block: &sunrise_items::blocks::StateFacts,
    branch: sunrise_world::level::UseBranch,
) -> bool {
    branch == sunrise_world::level::UseBranch::WithoutItem
        && [
            "AbstractBedBlock",
            "LightBlock",
            "RespawnAnchorBlock",
            "SignBlock",
        ]
        .iter()
        .any(|class| block.has_class(class))
}
impl GameLevel<'_> {
    /// `LilyPadBlock.entityInside`: the boxes of the boats that can break a pad.
    fn boat_trigger_boxes(&self) -> impl Iterator<Item = Aabb> + '_ {
        self.entities().boats.iter().map(boats::Boat::bounds)
    }
    /// How many entities in `area` press plates and tripwires, or with
    /// `LIVING` only the living ones; a constant so a list of other kinds
    /// compiles to no loop at all.
    fn count_triggers<const LIVING: bool>(&self, area: Aabb) -> usize {
        use entity::{Entity as _, for_each_entity};
        let mut count = 0;
        for_each_entity!(self.entities(), entity => {
            if (!LIVING || entity.is_living())
                && entity.triggers_blocks()
                && area.intersects(entity.bounds())
            {
                count += 1;
            }
        });
        count
    }
    /// Every entity box that blocks building: the players who aren't
    /// spectators, the minecarts and the `Blockers` (a removed mob or a marker
    /// armor stand, whose box is empty, doesn't).
    fn building_blocker_boxes(&self) -> Vec<Aabb> {
        let boxed = |(lo, hi): &([f64; 3], [f64; 3])| {
            Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            )
        };
        let b = self.entities();
        self.player_box
            .iter()
            .chain(
                self.building_player_boxes
                    .as_ref()
                    .unwrap_or(&self.guest_boxes),
            )
            .map(boxed)
            .chain(b.mobs.iter().filter(|m| !m.removed).map(Mob::bounds))
            .chain(
                b.non_mob_building_blocker_boxes(self.cart_boxes.iter().map(boxed), self.crystals),
            )
            .collect()
    }
}

impl<'w> entity::EntityLists<'w> {
    /// Non-player, non-mob boxes with `Entity.blocksBuilding`. The block level
    /// supplies its minecart snapshot; the mob level reads the live cart boxes.
    pub(in crate::game) fn non_mob_building_blocker_boxes(
        self,
        carts: impl Iterator<Item = Aabb> + 'w,
        crystals: &'w [dragon::Crystal],
    ) -> impl Iterator<Item = Aabb> + 'w {
        carts
            .chain(
                self.armor_stands
                    .iter()
                    .filter(|s| !s.marker)
                    .map(|s| s.bounds()),
            )
            .chain(self.boats.iter().map(boats::Boat::bounds))
            .chain(crystals.iter().map(dragon::Crystal::bounds))
            .chain(self.tnts.iter().map(tnt::PrimedTnt::bounds))
            .chain(
                self.falling
                    .iter()
                    .filter(|f| f.blocks_building)
                    .map(FallingBlock::bounds),
            )
    }
}
/// A landing's block game event, emitted after its world write is applied.
struct LandingVibration {
    event: &'static str,
    position: BlockPos,
    state: BlockStateId,
    from_entity: bool,
}

/// FallingBlockEntity carrying a block state.
#[derive(Clone, Debug)]
struct FallingBlock {
    attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    /// `Entity.yRot` and `xRot` (`/tp`, `/data get ... Rotation`).
    pub rotation: [f32; 2],
    /// When it joined its list (`sunrise_client::join_seq`); set by the list.
    pub(super) join: sunrise_client::join_seq::JoinSeq,
    /// Its entity id: from the entity counter on the server, the server's on a client.
    id: u32,
    state: BlockStateId,
    /// `FallingBlockEntity.fall` and `recreateFromPacket` set `blocksBuilding`;
    /// constructing an entity from NBT leaves the default false.
    blocks_building: bool,
    /// `DATA_START_POS`: spawn position used to select weighted block models.
    start_pos: BlockPos,
    position: Vec3,
    previous: Vec3,
    velocity: Vec3,
    /// `FallingBlockEntity.time`: signed, a save can start it below zero.
    age: i32,
    /// `Entity.isNoGravity`.
    no_gravity: bool,
    /// `FallingBlockEntity.tick` handles portals after moving, before landing.
    portal: sunrise_client::nether_portal::PortalState,
    /// `FallingBlockEntity.dropItem`.
    drop_item: bool,
    /// `Entity.fallDistance` (powder snow holds a block that fell far at 0.9).
    fall_distance: f64,
    /// `Entity.onGround`: whether its last move was stopped going down.
    ground: sunrise_world::ground_support::GroundState,
    /// `Entity.remainingFireTicks` (`Fire`): lava sets it, and it never counts
    /// down, as `FallingBlockEntity.tick` runs no `baseTick`.
    fire_ticks: i32,
    /// `Entity.syncVelocity`, set by `markHurt` (lava's hurt): a server sends
    /// its motion to the clients tracking it after its next update.
    sync_velocity: bool,
    /// `Entity.stuckSpeedMultiplier`: a cobweb it is in scales its next move.
    stuck: Vec3,
    damage: sunrise_client::falling_block::Damage,
    /// The UUID and compound it was loaded with, kept while it moves.
    saved: Option<Box<([i32; 4], sunrise_core::nbt::Tag)>>,
    /// What `entity_size` answered at its last size refresh (`EntityEvent.Size`): its box takes
    /// the answer while it holds for its vanilla size.
    pub(super) size_answer: Option<sunrise_client::pose::SizeAnswer>,
}
sunrise_mod_runtime::impl_attachment_lifetime!(FallingBlock);

impl FallingBlock {
    /// `EntityType.FALLING_BLOCK.getDimensions()`: width, height and eye height.
    const DIMENSIONS: [f32; 3] = [0.98, 0.98, 0.833_000_06];
    /// Its box (`falling_block::bounds`), or its `entity_size` answer's.
    fn bounds(&self) -> sunrise_core::geometry::Aabb {
        let vanilla = sunrise_client::falling_block::bounds(self.position);
        sunrise_client::pose::SizeAnswer::sized_box(
            self.size_answer,
            Self::DIMENSIONS,
            self.position,
            vanilla,
        )
    }
    /// Where `FallingBlockRenderer` draws the block's model corner, `alpha`
    /// into the tick: centred on the entity, then moved by `offset`, the
    /// block's random model offset (`ModelBlockRenderer.tesselateBlock`:
    /// `BlockState.getOffset` of `BlockPos.containing(x, maxY, z)`), so a
    /// pointed dripstone keeps the place it hung at while it falls.
    fn model_origin(
        &self,
        alpha: f64,
        offset: impl Fn(BlockStateId, i32, i32) -> Option<[f64; 3]>,
    ) -> glam::DVec3 {
        let [dx, dy, dz] = offset(
            self.state,
            self.position.x.floor() as i32,
            self.position.z.floor() as i32,
        )
        .unwrap_or([0.0; 3]);
        glam::DVec3::new(
            self.previous.x + (self.position.x - self.previous.x) * alpha - 0.5 + dx,
            self.previous.y + (self.position.y - self.previous.y) * alpha + dy,
            self.previous.z + (self.position.z - self.previous.z) * alpha - 0.5 + dz,
        )
    }
}

/// `FallingBlockRenderer.shouldRender` drops a falling block whose own state is
/// the state of the cell it is in (`level.getBlockState(entity.blockPosition())`):
/// a `NoGravity` block, or one that has just landed, sits inside an identical
/// block, and vanilla draws no model for it.
fn falling_block_visible(state: BlockStateId, cell: BlockStateId) -> bool {
    state != cell
}

/// The data loaded once at startup and never changed: blocks, items,
/// recipes, biomes, mobs, collision and motion. Read-only and `Send + Sync`,
/// so the client and an integrated server thread can share one copy.
pub struct Resources {
    core: WorldCore,
    rest: std::sync::OnceLock<WorldRest>,
}

struct WorldCore {
    item_data: Arc<sunrise_world::item_defaults::ItemData>,
    facts: BlockFacts,
    biomes: Arc<Biomes>,
    environment: Environment,
    collision: Arc<BlockCollisionRegistry<'static>>,
    motion: MotionResources,
    suffocation: sunrise_world::suffocation::SuffocationRegistry<'static>,
    variants: Arc<sunrise_client::mobs::variants::VariantTables>,
    threads: usize,
}

struct WorldRest {
    items: Arc<Items>,
    recipes: Recipes,
    /// The open world's `block_transformer` registry, published by
    /// [`Resources::set_transformers`] (empty until a world opens).
    transformers: std::sync::RwLock<Arc<sunrise_client::block_transform::Transformers>>,
    mob_types: HashMap<String, Arc<MobType>>,
    /// The registry keys of the player, the armor stand and the dragon, resolved with these
    /// resources as each mob type's own key is ([`MobType::type_key`]).
    living_type_keys: sunrise_client::health::LivingTypeKeys,
    mob_data: sunrise_client::mobs::MobData,
    /// Complete pre-creation facts for the registered entity types.
    spawner_types: HashMap<String, sunrise_client::mobs::spawner::SpawnerTypeFacts>,
    providers: Providers,
    block_entity_render: BlockEntityRenderer,
    animate: sunrise_client::animate_tick::AnimateTables,
    fog: sunrise_client::fog::FogTables,
    music: sunrise_client::music::MusicTables,
    ambient: sunrise_client::ambience::AmbientTables,
    nether_environment: Environment,
    end_environment: Environment,
    behaviours: Arc<Behaviours>,
    /// The player model's right and left arms.
    player_arm: [Option<sunrise_client::mobs::Part>; 2],
    /// The slim model's right and left arms (3 pixels wide).
    player_arm_slim: [Option<sunrise_client::mobs::Part>; 2],
    player_type: Option<Arc<sunrise_client::mobs::MobType>>,
    /// `SpinAttackEffectModel` (Riptide's spinning boxes).
    spin_attack: Option<sunrise_client::mobs::Part>,
    stations: sunrise_client::stations::StationData,
    test_environments: Arc<HashMap<String, test_environments::Environment>>,
    test_instances: Arc<HashMap<String, test_instances::TestDef>>,
}

/// The content tables a world's data packs change, loaded when the world
/// opens and on `/reload`: `ReloadableServerResources` loads a server's
/// recipes, loot tables and tags from the world's pack stack, over the
/// resource root's own. Worlds without packs use `Resources.rest` directly.
struct WorldTables {
    conditions: sunrise_world::conditions::LoadConditions,
    suffocation: sunrise_world::suffocation::SuffocationRegistry<'static>,
    /// The pack files the tables were built from (a native mod's among
    /// them), for the advancement tree reload and the loot data.
    files: Vec<(String, Vec<Vec<u8>>)>,
    priority_layers: Vec<sunrise_world::loot::RecipePriorityLayer>,
    items: Arc<Items>,
    /// Registry definitions stay frozen at world open, even when tags reload.
    packed_registries: Arc<sunrise_client::lan::data::PackRegistryDefinitions>,
    painting_variants: paintings::PaintingVariants,
    /// `block_transformer`, bound to the block registry.
    transformers: Arc<sunrise_client::block_transform::Transformers>,
    /// Frozen `minecraft:timeline` entries and their bound tags, read by `Environment` and
    /// `/time`.
    timelines: sunrise_client::environment::TimelineTable,
    /// Data-driven mob variant and sound-variant registries, frozen at world open.
    variants: Arc<sunrise_client::mobs::variants::VariantTables>,
    providers: Providers,
    biomes: Arc<world_loading::WorldBiomeRegistry>,
    loot: Arc<sunrise_world::loot::LootData>,
    loot_modifier_manager: Arc<mod_api::GlobalLootModifierManager>,
    recipes: Recipes,
    stations: sunrise_client::stations::StationData,
    reload_sources: Option<Arc<sunrise_client::crafting::ResolvedSources>>,
    /// The values of the data map types native mods registered (`native_data_map_types`).
    data_maps: native_data_maps::DataMaps,
    /// Pack tags used by advancement predicates and built-in data maps
    /// (`TagLoader.loadTagsForExistingRegistries`). Damage tags live in the catalog.
    pack_tags: HashMap<&'static str, HashMap<String, HashSet<String>>>,
    /// The gameplay attributes of the dimension types, biomes and timelines (with the
    /// packs' timeline tags) that the raid code reads.
    gameplay: Arc<sunrise_client::patrol::GameplayAttributes>,
    /// `Registries.TEST_ENVIRONMENT` and `Registries.TEST_INSTANCE`, frozen at world open.
    test_environments: Arc<HashMap<String, test_environments::Environment>>,
    test_instances: Arc<HashMap<String, test_instances::TestDef>>,
}

struct PreparedParts {
    block_states: Arc<api_values::block_state::Index>,
    parts: Option<mod_api::Parts>,
    cauldrons: Arc<cauldron_interactions::Table>,
}
use condition_reload_state::PreparedWorldReload;

impl WorldTables {
    /// The registries [`WorldTables::pack_tags`] holds.
    const PACK_TAG_REGISTRIES: [&'static str; 2] = ["worldgen/biome", "worldgen/structure"];
}

/// The equipment slot of `armor[index]`: head, chest, legs, feet.
/// `doHurtEquipment`'s `durabilityDamage` before the cast to int: `max(1, damage / 4)`.
pub(super) fn durability_damage(damage: f32) -> f32 {
    (damage / 4.0).max(1.0)
}
/// The durability damage a mod left for `slot` (`mod_api::ArmorHurt::new_damage`, cast to int
/// as vanilla casts it); an entry it removed keeps vanilla's `fallback`.
pub(super) fn answered_wear(
    entries: &[mod_api::ArmorHurt],
    slot: sunrise_core::equipment_slot::EquipmentSlot,
    fallback: u32,
) -> u32 {
    entries
        .iter()
        .find(|e| e.slot == slot)
        .map_or(fallback, |e| e.new_damage as u32)
}

pub(super) fn armor_slot(index: usize) -> sunrise_core::equipment_slot::EquipmentSlot {
    use sunrise_core::equipment_slot::EquipmentSlot;
    match index {
        0 => EquipmentSlot::Head,
        1 => EquipmentSlot::Chest,
        2 => EquipmentSlot::Legs,
        _ => EquipmentSlot::Feet,
    }
}

pub use resource_tables::UiResources;

/// What a resource pack reload replaces (`Minecraft.reloadResourcePacks`):
/// block models and sprites with their meshing tables, item models,
/// translations and the textures read on the CPU. Each game holds its own;
/// cloning shares the loaded data.
#[derive(Clone)]
pub struct PackResources {
    source: sunrise_assets::resource_source::ResourceSource,
    data_source: sunrise_assets::resource_source::ResourceSource,
    data_changes: sunrise_assets::resource_source::ResourceSource,
    additional_data: Arc<[Arc<sunrise_client::datapack::DataPack>]>,
    mod_layers: Arc<[sunrise_assets::resource_source::ResourceSource]>,
    assets: Arc<Assets>,
    /// `Blocks.WATER.defaultBlockState()` for screen-effect extraction.
    water_overlay_state: mod_api::BlockState,
    block_registries: Arc<sunrise_client::command::args::Registries>,
    render: Arc<RenderAssets>,
    context: Arc<WorkerContext>,
    layer_names: Arc<HashMap<u32, String>>,
    /// Item definitions, baked item models and special renderers.
    item_renderer: Arc<sunrise_client::item_render::ItemRenderer>,
    names: Arc<Names>,
    /// clouds.png: (width, height, filled cells).
    clouds: Arc<(u32, u32, Vec<bool>)>,
    /// misc/forcefield.png: (width, height, RGBA) for the world border wall.
    forcefield: Arc<(u32, u32, Vec<u8>)>,
    /// Custom sky layers (OptiFine's `optifine/sky`) with their textures; none without them.
    custom_sky: Arc<dyn sunrise_client::custom_sky::SkyLayers>,
    /// The overworld's, Nether's and End's attribute stacks with the packs' Polytone biome and
    /// dimension modifiers (#11486), when a pack has some: what the frame draws.
    environments: Option<Arc<[Environment; 3]>>,
    /// The packs' Polytone custom particles and block emitters (#11486), when a pack has some.
    polytone_particles: Option<Arc<sunrise_client::particles::polytone::Registry>>,
    /// The packs' Polytone post chains (#11486), when a pack has some.
    polytone_post: Option<Arc<sunrise_client::polytone_post::PolytonePost>>,
    /// The packs' Polytone custom colours (`polytone/colors.json`).
    polytone_colors: Arc<sunrise_client::pack_palette::PackColors>,
}

/// A quick craft drag as the screen draws it: each dragged slot's count and
/// whether the slot caps it (None: hidden), and what stays in hand.
struct DragView {
    slots: HashMap<usize, Option<(u32, bool)>>,
    left: Option<u32>,
}
fn equipment_modifier_applies(group: &str, slot: &str) -> bool {
    group == slot
        || group == "any"
        || (group == "armor" && slot != "mainhand" && slot != "offhand")
        || (group == "hand" && (slot == "mainhand" || slot == "offhand"))
}
/// Ctrl+Q reads the physical keyboard, independently of sprint key mappings.
/// A chunk whose ticking future can complete (`ChunkMap.prepareTickingChunk`):
/// lit, with its neighbours loaded, as `postprocess_chunks` waits for.
fn ticking_chunk(world: &World, x: i32, z: i32) -> bool {
    world.chunks.is_ticking_ready((x, z))
}
/// `TicketStorage.canTicketExpire` for a `PORTAL` ticket: its chunk's holder is
/// ready for saving once the futures of its level are done, the chunk lit and
/// its neighbours loaded out to one chunk at a ticking level
/// (`prepareTickingChunk`) and two at an entity-ticking one
/// (`prepareEntityTickingChunk`). The ticket's own level stands for the
/// holder's: the area it waits for is the one its own streaming loads.
fn portal_ticket_can_expire(
    world: &World,
    ticket: &sunrise_client::ticket_levels::PortalTicket,
) -> bool {
    use sunrise_client::ticket_levels::{BLOCK_TICKING, ENTITY_TICKING};
    let r = match ticket.level {
        l if l <= i32::from(ENTITY_TICKING) => 2,
        l if l == i32::from(BLOCK_TICKING) => 1,
        _ => 0,
    };
    let (x, z) = ticket.chunk;
    world.chunks.get(&(x, z)).is_some_and(|c| c.lit)
        && (-r..=r).all(|dz| (-r..=r).all(|dx| world.chunks.contains_key(&(x + dx, z + dz))))
}
/// Key events carry the scancode's US-layout code, which ignores XKB options like
/// `caps:swapescape`. For the keys whose meaning the keymap may remap, take the code
/// from the keysym the keymap resolved instead. This includes function keys on
/// remapped media rows; ordinary character bindings still use their physical key.
fn follow_keymap(event: &mut WindowEvent) {
    if let WindowEvent::KeyboardInput { event, .. } = event {
        follow_named_keymap(&event.logical_key, &mut event.physical_key);
    }
}

fn select_creative_tab(
    creative: &mut Creative,
    clicks: &mut sunrise_client::container_screen::Clicks,
    tab: usize,
) {
    creative.select(tab);
    *clicks = Default::default();
    // Tabs are outside the panel; their release must not drop a carried stack.
    clicks.skip_release();
}

/// The icon the Select World list shows for the world in play
/// (`GameRenderer.tryTakeScreenshotIfNeeded`): its path in the save, and when
/// the last frame grab was asked for.
struct WorldIcon {
    path: PathBuf,
    last_attempt: Option<Instant>,
}

#[derive(Debug, PartialEq)]
struct QueuedLevelEvent {
    kind: i32,
    pos: [i32; 3],
    data: i32,
    source: Option<sunrise_world::level::LevelEventPlayer>,
}

/// Physical and scaled GUI dimensions last reported by `window_resize`.
#[derive(Clone, Copy, PartialEq)]
struct ResizeState {
    size: winit::dpi::PhysicalSize<u32>,
    gui_size: [u32; 2],
}

impl ResizeState {
    /// Records `self` as the last report and says whether it differs from the previous one.
    fn differs_from(self, last: &mut Option<Self>) -> bool {
        last.replace(self) != Some(self)
    }
}

/// Temporary block-contact collections rebuilt on every server tick. Keeping
/// their backing storage on `Game` avoids reallocating them while the world is
/// ticking; the wrapper around `tick_blocks` restores the scratch even when a
/// phase returns an error.
#[derive(Default)]
struct TickScratch {
    ticking: Vec<(i32, i32)>,
    inside: Vec<BlockPos>,
    standing: Vec<(BlockPos, bool, f64)>,
    boat_cells: Vec<BlockPos>,
    touched: Vec<BlockPos>,
    sensing: Vec<BlockPos>,
    rest: Vec<BlockPos>,
    later: Vec<BlockPos>,
}

/// Video options a mod set for this session only, over the player's saved ones.
#[derive(Default)]
struct SessionVideo {
    vsync: Option<bool>,
    /// As `options.txt` names it: `false`, `fast` or `true`.
    clouds: Option<&'static str>,
    view_bobbing: Option<bool>,
}

struct Game<'a> {
    /// Shared with an integrated server; `collision` and `motion` borrow from it.
    res: Arc<Resources>,
    rest_loader: Option<WorldRestLoader>,
    pending_entry: Option<PendingWorldEntry>,
    /// `DebugEntryChunkGeneration`'s `lastPos` and `result`: F3's `Density` line is sampled
    /// again only when the feet block (or the dimension) changes.
    debug_density: RefCell<debug_keys::DensityCache>,
    /// What each mod added to F3's frame rate line (`set_debug_fps_text`), by mod id.
    debug_fps_texts: RefCell<Vec<(&'static str, String)>>,
    pending_pack_reload: bool,
    /// HDR, Smooth Gradients, picked path and outcome at the last frame attempt, including a failed
    /// replacement (`follow_frame`).
    frame_attempted: render_settings::FrameAttempt,
    /// When `resourcepacks/` was last looked at for the open pack screen, and what it held
    /// (`watch_pack_folder`).
    pack_folder: Option<(
        std::time::Instant,
        Vec<(String, u64, Option<std::time::SystemTime>)>,
    )>,
    /// A pack reload staged on a worker until `drive_pack_reload` applies it.
    pack_reload: Option<packs::PackReload>,
    /// Its `LoadingOverlay`, up from the reload's start until it has faded
    /// out after it: no input reaches the game meanwhile.
    reload_overlay: Option<packs::ReloadOverlay>,
    /// A server's resource packs (game/world/server_packs.rs).
    server_packs: server_packs::ServerPackState,
    /// The window's inbox for server notices, kept across world disconnects.
    client_reports: Arc<sunrise_client::chunk_io::Reports>,
    /// The link a chat click asks to open (`ConfirmLinkScreen`).
    link_prompt: Option<String>,
    /// A command the player didn't type, waiting for their answer
    /// (game/network/remote_chat.rs).
    command_prompt: Option<remote_chat::CommandPrompt>,
    /// A server's Code of Conduct to accept (game/interface/code_of_conduct.rs).
    code_of_conduct: Option<code_of_conduct::Prompt>,
    dialogs: dialogs::DialogState,
    /// Single player's server while it loads its world (`Minecraft.doWorldLoad`).
    integrated_starting: Option<integrated::Starting>,
    world_renderer_ready: bool,
    collision: &'a BlockCollisionRegistry<'a>,
    motion: &'a MotionResources,
    /// This world's frozen damage definitions and reloadable tag snapshot.
    pub(super) health_resources: sunrise_client::health::HealthResources,
    /// Contact hurts a vehicle tick queued; applied once its copy is stored.
    pub(super) vehicle_contact_hurts: Vec<crate::game::entity::ContactHurt>,
    damage_tag_generation: u64,
    /// The registries native mods registered data map types for, as of the
    /// server tables' last build: what `data_maps_updated` fires for.
    data_map_registries: Vec<String>,
    /// This game's resource packs (`reload_packs` replaces them).
    pending_initial_models: Option<Box<initial_client_models::PendingClientStart>>,
    stitching_block_atlas: Option<Arc<RenderAssets>>,
    pack: PackResources,
    /// The title and menu bundle (GUI atlas, font and panorama).
    gui: UiResources,
    instance_groups: std::cell::RefCell<native_instance_groups::InstanceGroupStorage>,
    prepared_items: std::cell::RefCell<native_instance_groups::PreparedItemStorage>,
    standalone_geometry: native_block_entity_draw::StandaloneGeometry,
    /// `FogRenderer`'s evolving state.
    fog: sunrise_client::fog::FogState,
    screen_fx: sunrise_client::screen_effects::ScreenEffects,
    /// `LocalPlayer.itemActivation`: the totem flying up to the screen.
    item_activation: sunrise_client::item_activation::ItemActivation,
    weather_parameters: sunrise_client::weather::ParameterCache,
    panorama_rotation: sunrise_client::gfx::panorama::Rotation,
    world: World,
    /// The biome table the world plays with: the game's own with the world's
    /// packs over it (`Biomes::with_packs`), `None` before a world opens.
    world_biomes: Option<Arc<Biomes>>,
    /// `world_biomes` as the biome files made it, before `native_biome_colors`.
    world_biomes_base: Option<Arc<Biomes>>,
    streaming: Streaming,
    lod: Option<sunrise_client::lod::Lod>,
    lod_source_attempted: bool,
    lod_observe: sunrise_client::fast_hash::FxHashSet<(i32, i32)>,
    /// The server level far terrain keeps the chunks of.
    lod_level: Option<String>,
    /// The server's BlueMap far terrain draws from, when the player named one.
    lod_blue_map: Option<Arc<sunrise_client::bluemap::BlueMap>>,
    /// The detail revision, centre and radius `Lod::update` last had covered for.
    lod_covered_at: Option<(u64, (i32, i32), i32)>,
    /// Whether the view's columns have all arrived, and how new it is: large
    /// translucent pages wait for both before they shrink (`set_terrain_loading`).
    view_arrival: sunrise_client::streaming::ViewArrival,
    options: Options,
    player: Option<PlayerMotion>,
    drop_spam_throttler: sunrise_client::menu::DropSpamThrottler,
    spawn: Option<Vec3>,
    clock: FixedStep,
    previous_position: Vec3,
    /// `yRotO` / `xRotO` of the local player: the view at the start of the tick,
    /// moved by `Entity.turn` and a boat's `clampRotation` as the view is
    /// (`LocalPlayer.getRopeHoldPosition` eases the held lead from it).
    rope_rot_old: (f32, f32),
    eye: f32,
    previous_eye: f32,
    yaw: f32,
    pitch: f32,
    inventory: Vec<Option<ItemStack>>,
    selected: usize,
    carried: Option<ItemStack>,
    craft_grid: Vec<Option<ItemStack>>,
    craft_size: usize,
    /// The crafting table the 3x3 grid belongs to (`CraftingMenu.access`).
    craft_table: Option<[i32; 3]>,
    /// A quick craft drag over inventory slots: (button, game slots so far).
    inv_drag: Option<(i32, Vec<usize>)>,
    /// The mouse buttons whose press a container hook took: their release is the hooks' too.
    container_buttons: HashSet<MouseButton>,
    /// This mouse event's button-up ended a container hook's press latch (`container_button_up`).
    container_owned_up: bool,
    /// Physical mouse presses so far: a drag knows whether its press is still the latest.
    physical_presses: u64,
    /// `AbstractContainerScreen`'s quick craft and skip-release state.
    inv_clicks: sunrise_client::container_screen::Clicks,
    /// The Mouse Tweaks extra's drag and wheel (`tweaks`).
    tweaks: extras::mouse_tweaks::Tweaks,
    /// The wheel and hovered slot of `BundleMouseActions`.
    bundle_mouse: bundles::BundleMouse,
    /// The last inventory click, for the double click that gathers a stack.
    last_inv_click: Option<(std::time::Instant, Option<usize>, bool)>,
    /// `lastQuickMoved`: what the last Shift click moved, which a Shift
    /// double click moves every stack of.
    last_quick_moved: Option<ItemStack>,
    /// Open container block entity, if any.
    container: Option<(i32, i32, i32)>,
    /// The open container and the slots it had when it opened:
    /// `Container.stillValid` holds only while every block entity the menu
    /// took is still there, so a half of a double chest (or the container)
    /// going away closes it.
    open_container_slots: Option<((i32, i32, i32), usize)>,
    /// Open workstation menu (enchanting table, anvil, ...).
    station: Option<stations::Station>,
    /// Status effects and thrown potions (game/items/potions.rs).
    potions: potions::PotionState,
    scratch_slot: Option<ItemStack>,
    mining: Option<Mining>,
    /// The other players' cracks (`LevelRenderer.destroyingBlocks`): breaker id to the block,
    /// stage (0..=9), last refresh time and stored additional display positions.
    destroying: HashMap<i32, client_cracks::ClientCrack>,
    destroy_delay: u32,
    use_delay: u32,
    creative: bool,
    /// The game mode; `creative` mirrors `mode == Creative`.
    mode: sunrise_client::command::args::GameMode,
    keys: sunrise_client::held::HeldKeys,
    // Physical input is separate from remapped and toggled movement keys.
    physical_keys: HashSet<KeyCode>,
    /// Keys whose first press a mod cancelled (`key_mapping_pre`): their repeats are never held.
    rejected_keys: HashSet<KeyCode>,
    physical_buttons: HashSet<MouseButton>,
    drag_button: Option<MouseButton>,
    /// Control + left click as a right click (macOS).
    right_click_emulation: sunrise_client_input::mouse::RightClickEmulation,
    cursor: (f32, f32),
    /// The window pointer sample; frames and input project it at the current GUI size.
    window_cursor: Option<sunrise_client::window::PointerSample>,
    window_pointer_space: std::cell::OnceCell<sunrise_client::window::PointerSpace>,
    /// The cursor icon the window shows (`widgets::take_cursor` each frame).
    cursor_icon: sunrise_client::widgets::Cursor,
    captured: bool,
    /// Native grab mode used by winit. Linux window managers can drop the
    /// grab without a focus event, leaving `captured` true but stale.
    capture_mode: Option<CursorGrabMode>,
    /// The window held the pointer when it lost focus: the system lets a
    /// grabbed pointer go with the focus, and a window that comes back still
    /// wanting it takes it again (`MouseHandler.grabMouse`, whose grab
    /// outlives the window's focus).
    regrab_on_refocus: bool,
    /// The open mod screen holds the pointer (`ClientHost::set_pointer_grab`): `captured` is
    /// its grab, and the device's motion goes to `screen_pointer_motion`, not the camera.
    /// Cleared by any release and when the screen changes.
    screen_grab: bool,
    /// A grab or release the calling mod asked for through `&self` (`set_pointer_grab`),
    /// applied by `apply_pointer_grab_request` at the next input event or frame.
    pointer_grab_request: std::cell::Cell<Option<bool>>,
    /// Files dropped on the window in this event-loop pass, in the order winit gave them
    /// (one event each), kept only while a mod listens to `screen_files_dropped`; the pass's
    /// `idle` fires them as one list, as vanilla's `MouseHandler.onDrop` hands `Screen.onFilesDrop`.
    dropped_files: Vec<std::path::PathBuf>,
    /// Controller connection notices.
    controller_notice: sunrise_client::controller_notice::NoticeSlot,
    focused: bool,
    /// The pointer is over the window, as the window system last told (its enter and leave
    /// events, or a move inside it).
    window_hovered: bool,
    /// The window is fully hidden (`WindowEvent::Occluded`): covered, on another desktop, or
    /// minimized where the system reports that.
    window_occluded: bool,
    /// The visible window of a game an agent started, until a person clicks into it: its
    /// pointer grab stays inside the game as `--offscreen`'s does, so the person's pointer is
    /// never taken, hidden or moved, and their keys and mouse don't reach the game.
    unclaimed_window: bool,
    /// The modifiers the person last held at it, applied at the claim (`held_modifiers`).
    unclaimed_modifiers: Option<winit::event::Modifiers>,
    /// The window's last focus change during a scripted run, applied once the run ends.
    focus_after_run: Option<bool>,
    /// Whether the window currently accepts input method events.
    ime_allowed: bool,
    /// The character of the key held down, for the input method's echo of it.
    held_key_text: ime::HeldKeyText,
    /// The candidate window's area last given to the window, in pixels.
    ime_area: Option<[i32; 4]>,
    /// When the window lost focus, while it is still without
    /// (`Minecraft.lastActiveTime`: pausing waits 500 ms).
    focus_lost: Option<Instant>,
    /// The keyboard's modifier state (`KeyEvent.modifiers`).
    modifiers: winit::keyboard::ModifiersState,
    /// The window's fullscreen request and reported state.
    fullscreen: sunrise_client::window::FullscreenState,
    /// The monitor whose video modes `settings.video_modes` holds.
    video_modes: sunrise_client::window::VideoModeList,
    /// MouseHandler's accumulated look and wheel state.
    mouse: MouseLook,
    /// An agent moved the mouse (`mouse-move`) since the pointer was last
    /// grabbed or released: its motion turns the view like a grabbed pointer.
    agent_pointer: bool,
    /// `Options.smoothCamera` (cinematic camera), not persisted.
    smooth_camera: bool,
    /// Uniform `KeyState` (`isDown`, `clickCount`) for every vanilla mapping slot.
    vanilla_mappings: sunrise_client::keymapping::VanillaMappings,
    /// A world screen with the pointer captured was open last frame.
    was_playing: bool,
    /// Minecraft.pause: a single-player screen that pauses the game is open.
    pause: bool,
    debug: bool,
    /// `sunrise-ctl gpu-passes`: time Sunrise's own passes on the GPU with F3 shut.
    gpu_passes: bool,
    /// `sunrise-ctl gpu-passes detail`: also dispatch times and counters.
    gpu_pass_detail: bool,
    screen: Screen,
    screen_state: screen_build::ScreenState,
    window: Option<Arc<Window>>,
    /// What `window_resize` last reported: the physical size and the GUI settings behind the scaled size.
    resize_state: Option<ResizeState>,
    /// The renderer's mod GPU tables and device handles, set with the renderer by `set_renderer`.
    /// Declared before `renderer` so it drops first: its `Device` and `Queue` clones are gone
    /// before the renderer's surface is, at `Game` drop. Invariant: it is `Some` exactly when a
    /// renderer is installed or `Game::render` has it out; clear both through `clear_renderer`.
    pub(in crate::game) mod_gpu: Option<sunrise_client::gfx::ModGpuResources>,
    /// `None` while `Game::render` has the renderer out for `Renderer::render`, and so during every
    /// frame hook a mod runs. Nothing a hook reaches may read it: state facing mods lives in
    /// [`Self::mod_gpu`]'s `ModGpuResources`, and the per-frame values in `render_snapshot`.
    renderer: Option<Renderer>,
    /// Avoid an endless rebuild loop if the new device is lost as well.
    last_device_recovery: Option<Instant>,
    /// CPU geometry storage retained across frames, including empty/title frames.
    entity_vertices: Vec<Vertex>,
    /// Sunrise Extras' dynamic lights this frame (`dynamic_lights.rs`), and how brightly each
    /// item glows (built the first time the extra is on).
    dynamic_lights: Vec<[f32; 4]>,
    luminance: std::cell::OnceCell<sunrise_client::dynamic_light::Luminance>,
    /// Name tags drawn over everything (`RenderPipelines.TEXT_SEE_THROUGH`).
    see_through_vertices: Vec<Vertex>,
    /// The first-person player's model, which only a native pack's shadow pass draws.
    shadow_only_vertices: std::cell::RefCell<Vec<Vertex>>,
    /// Complete quads already committed to other streams in the active frame's combined entity
    /// batch (additive, hand, and entity vertices outside the current target buffer).
    frame_batch_reserved_quads: std::cell::Cell<usize>,
    /// Runs of `entity_vertices` by entity, for a shader pack's per-entity draws; recorded only
    /// while `pack_ids` is set (game/interface/shader_packs.rs).
    entity_spans: std::cell::RefCell<Vec<sunrise_client::gfx::pack_draw::EntitySpan>>,
    /// The runs of `entity_geometry`'s ordered pass, kept to reuse their storage.
    entity_runs: Vec<entity::DrawRun>,
    /// Which entity this frame's entity vertices belong to (`Frame::entity_motion`).
    entity_motion: std::cell::RefCell<Vec<(u32, u64)>>,
    /// The GPU-posed vertex count at each of `entity_motion`'s marks (`Frame::entity_motion_posed`).
    entity_motion_posed: std::cell::RefCell<Vec<u32>>,
    /// Mob models posed on the GPU (`Frame::skin`, `mobs::skin`): the models stored there,
    /// and this frame's draws.
    skin_models: std::cell::RefCell<sunrise_client::mobs::skin::SkinModels>,
    skin_frame: std::cell::RefCell<sunrise_client::gfx::entity_skin::SkinFrame>,
    /// The glowing entities' silhouettes this frame (game/entities/glowing.rs).
    glowing_vertices: std::cell::RefCell<Vec<Vertex>>,
    /// Client mods' sounds (`play_local_sound`): those waiting for the next audio update
    /// (`tick_sound_engine`), the stops and fades asked of them, and their handles.
    mod_sounds: native_local_sounds::ModSounds,
    /// A client mod asked to clear the particles (`clear_particles`), done before
    /// the next frame or particle tick (`apply_mod_particle_clear`).
    mod_clear_particles: std::cell::Cell<bool>,
    /// The level random a block part's `animate_tick` holds as `event.random`, while it runs:
    /// the game's own draws during the call go through it (`with_lent_level_random`).
    lent_level_random: Option<std::rc::Rc<std::cell::RefCell<mod_api::LegacyRandomSource>>>,
    /// The shader pack's entity and block entity ids, while one draws entities.
    pack_ids: Option<shader_packs::PackIds>,
    started: Instant,
    /// `SpectatorGui`: the menu a spectator opens with the hotbar keys.
    spectator_gui: sunrise_client::spectator_menu::SpectatorGui,
    previous: Instant,
    /// FramerateLimitTracker.latestInputTime.
    last_input: Instant,
    /// Controllers (Sunrise's own).
    pad: pad::Pads,
    frames: u64,
    /// The glint clock and options, read once per frame (game/interface/item_draw.rs).
    glint_frame: sunrise_client::item_render::GlintFrame,
    /// Mod block entities' kept geometry (game/modding/native_block_entity_draw.rs).
    kept_block_entities: std::cell::RefCell<native_block_entity_draw::KeptGeometry>,
    /// Mod screens' living scenes, per mod and clock id (game/modding/native_scene_live.rs).
    live_scenes: std::cell::RefCell<native_scene_live::LiveScenes>,
    /// Client mods' dynamic textures, one GPU texture each (game/modding/native_dynamic_textures.rs).
    dynamic_textures: sunrise_client::hud::dynamic_textures::DynamicTextures,
    /// Glyphs from `native_draw_text` (`&self`): uploaded to the font atlas with the frame.
    text_patches: std::cell::RefCell<Vec<sunrise_client::font::Patch>>,
    /// Prepared block-entity copies for this frame, in entity-stream order.
    prepared_model_draws: std::cell::RefCell<Vec<sunrise_client::gfx::PreparedModelDraw>>,
    /// Scratch for the copies a frame draws through the terrain materials.
    model_copy_order: std::cell::RefCell<Vec<u32>>,
    world_geometry: Option<sunrise_client::world_geometry::WorldGeometry>,
    /// Read-only renderer snapshot cached while `self.renderer` is temporarily taken for
    /// `Renderer::render` so `ClientHost` queries still see framebuffer size and
    /// prepared-model limits.
    render_snapshot: std::cell::Cell<Option<RenderSnapshot>>,
    /// The shader pack's state and bound mod pipeline slots as the latest frame's render
    /// callbacks read them, set before `Renderer::render` takes `self.renderer` away
    /// (`ClientOptionsClientHost::shader_pack_state`, `RenderAssets::pipeline_pack_bound`).
    pack_frame_state: std::cell::Cell<Option<mod_api::ShaderPackState>>,
    pack_frame_slots: std::cell::RefCell<Vec<usize>>,
    /// `DebugRenderer.renderers`: active mod renderers and their owners, resolved at refresh.
    debug_renderers: Vec<sunrise_client::debug_entries::ActiveDebugRenderer>,
    /// `DebugRenderer.lastDebugEntriesVersion`: none before the first view of a world.
    debug_renderers_version: Option<u64>,
    /// Next undrawn mod layer in HUD `gui_layers::order`; `None` outside their pass.
    hud_layer_next: Option<usize>,
    /// Drawn bounding rectangles `[x, y, width, height]` in GUI pixels for HUD layers drawn so far
    /// in the current HUD pass (`ScreensClientHost::hud_layer_bounds`). Cleared before screens draw.
    hud_layer_bounds: Vec<(&'static str, [f32; 4])>,
    /// The status columns' heights in this HUD pass (NeoForge's `Hud.leftHeight`, `rightHeight`).
    hud_status_heights: hud_layout::StatusBarHeights,
    /// Heights a mod set during the current HUD layer (`set_gui_status_heights`), applied to
    /// `hud_status_heights` before the next body draws. A cell, as mods write through `&self`.
    hud_status_heights_set: std::cell::Cell<Option<[i32; 2]>>,
    /// The mod block entities' view distances, asked of the parts once (`native_block_entity_draw`).
    mod_view_distances: std::cell::OnceCell<native_block_entity_draw::ViewDistances>,
    /// What drawing a block entity asks of its block, by state id (`block_entity_reach`):
    /// resolved from the block names once, not for every block entity in every frame.
    block_entity_reach: std::cell::OnceCell<Box<[(Reach, bool)]>>,
    /// `item_level` for one frame, game time, day time and dimension.
    item_level_cache: std::cell::RefCell<
        Option<(
            (u64, i64, i64, &'static str),
            sunrise_assets::item_model::LevelState,
        )>,
    >,
    /// Frames drawn with the player in the world, for `--world-frames`.
    world_frames: u64,
    fps: f32,
    fps_frames: u64,
    fps_at: Instant,
    ticks: u64,
    /// `Hud.tickCount`, advanced on each unpaused client tick, including in menus.
    gui_ticks: i32,
    /// The sprite animation clock (`TextureManager.tick`): counts only the ticks the level
    /// runs normally, so `/tick freeze` holds it and `/tick step` moves it on.
    texture_ticks: u64,
    health_hud: hud_layout::HealthHud,
    /// What ends the game with an error; kept as it is, so main can tell a debug crash.
    error: Option<Box<dyn std::error::Error>>,
    message: Option<(String, Instant)>,
    save: Arc<SaveDir>,
    level: Level,
    /// Must precede session_lock: dropping the writer drains writes while the world is locked.
    player_autosaves: sunrise_client::lan::playerdata::Autosaves,
    session_lock: Option<sunrise_client::save::SessionLock>,
    /// The prepared world is installed. The session lock is taken earlier so tag
    /// reads see ready tables, and is not this marker: a final save before the
    /// prepared `Level` is installed would replace the world with placeholder state.
    world_adopted: bool,
    /// An early shutdown must not replace on-disk tickets while the server is
    /// still publishing levels or activating their tickets.
    startup_ticket_restore_pending: bool,
    audio: Audio,
    /// The side, registries and mod settings of this game's resource generation, which
    /// its menus lend to the hooks they fire inside a loop; rebuilt by `refresh_loop_context`.
    loop_context: Arc<sunrise_world::native_load_context::LoopContext>,
    /// This thread's authority over the game's tick-owned state (`parallel::TickThread`): a
    /// `Game` never leaves the thread that built it.
    tick: sunrise_client::parallel::TickThread,
    /// Authoritative store for this game side's native mod settings registered with
    /// `SettingsRegistrar::register_setting`.
    native_settings: native_hooks::mod_settings::SharedSettings,
    /// The native damage hooks every level of this game lends to its hits.
    damage_hooks: sunrise_client::health::DamageHooks,
    paused_mob_hurt: Option<Box<sunrise_client::mobs::PausedMobHurt>>,
    music: sunrise_client::music::MusicManager,
    /// The tracks native mods play over the music (`native_music_layers`).
    music_layers: sunrise_client::music::MusicLayers,
    /// `ToastManager.nowPlayingToast`: there while Music Toast is not Never.
    now_playing: Option<sunrise_client::toasts::NowPlayingToast>,
    /// The Music Toast option the toast was last set up for.
    music_toast_state: u8,
    music_notes: sunrise_client::toasts::MusicNotes,
    ambience: sunrise_client::ambience::Ambience,
    /// The sound engine was told the game paused (pauseAllExcept).
    /// The sounds' pause as last applied: (the game's pause, a mod's `set_sounds_paused`).
    sound_paused: (bool, bool),
    /// A client mod paused every sound (`set_sounds_paused`).
    mod_sounds_paused: bool,
    /// Minecarts whose `MinecartSoundInstance` is playing.
    cart_sounds: HashSet<u32>,
    /// The minecart the riding sounds were started for (`LocalPlayer.startRiding`).
    riding_cart_sounds: Option<u32>,
    /// The happy ghast its riding sound was started for (`LocalPlayer.startRiding`).
    riding_sound: Option<u32>,
    /// `LocalPlayer.wasFallFlying` for the elytra's flying sound.
    was_fall_flying: bool,
    /// Bees whose buzz is playing, and whether it is the aggressive one.
    bee_sounds: HashMap<u32, bool>,
    /// Remote guardian eye targets reused between ticks.
    guardian_look_scratch: Vec<(usize, [f64; 3])>,
    dropped: sunrise_client::join_seq::JoinList<ItemEntity>,
    /// The lethal damage type recorded while a lifted dropped item died with a listening
    /// `ItemDestroyed` hook, consumed by its caller once `dropped` is put back.
    item_destroyed_damage: Option<(u32, String)>,
    next_entity: u32,
    /// The last entity id when this level's entity pass began: an item with a
    /// later one was added during that pass and first ticks on the next
    /// (`EntityTickList`). `u32::MAX` until a tick has run.
    ticked_entities_through: u32,
    /// The items `tick_blocks` dropped while applying the entities' block
    /// contacts (a boat breaking a lily pad). Vanilla runs those in each
    /// entity's own tick (`applyEffectsFromBlocks`), inside the entity pass,
    /// so they too first tick on the next tick.
    contact_drops: std::ops::Range<u32>,
    particles: sunrise_client::particles::ParticleEngine,
    explicit_particle_quads: sunrise_client::particles::ParticleQuads,
    /// `ItemPickupParticle`s: taken items, orbs and arrows flying to their collector.
    pickups: pickups::Pickups,
    /// Sign text being edited (`SignEditScreen`).
    sign_edit: Option<sign_edit::SignEditor>,
    /// An open book, book and quill or lectern screen.
    book: Option<books::BookScreen>,
    /// The last click on the book screen, for double clicks.
    book_last_click: Option<std::time::Instant>,
    /// Lecterns whose page-turn pulse is running (ticks left).
    lectern_pulses: Vec<((i32, i32, i32), i32)>,
    /// LevelLoadingScreen.smoothedProgress while the player is not placed yet.
    loading_progress: f32,
    /// `createWorld.preparing` has been drawn: the data read runs on the next poll.
    creation_read_started: bool,
    /// When `loading_progress` last eased (the server thread's load, 50 ms a step).
    loading_progress_at: Option<std::time::Instant>,
    /// A received packet started or ended a piston's move: `poll_remote`
    /// syncs the moving pistons once its packets are handled, not once per packet.
    moving_pistons_stale: bool,
    /// The world spawn search's origin chunk and fallback height, once known.
    spawn_origin: Option<((i32, i32), i32)>,
    /// `native_create_spawn_position` was asked, and a mod chose the spawn.
    spawn_hook_fired: bool,
    spawn_hook_cancelled: bool,
    spawn_spiral: sunrise_client::world_spawn::ChunkSpiral,
    /// A new player's spawn search, while its candidates' chunks load.
    player_spawn: Option<sunrise_client::world_spawn::PlayerSpawnFinder>,
    /// Chunks a respawn search waits for this tick, kept loading beside the
    /// player's own (taken by `update_world`).
    spawn_wait: Vec<(i32, i32)>,
    mobs: sunrise_client::join_seq::JoinList<Mob>,
    /// `mobs` by network id, for the packets that name one (`game/mobs/mob_index.rs`).
    mob_index: mob_index::MobIndex,
    creatures: creatures::Creatures,
    sculk: sculk::SculkState,
    raiding: raids::Raiding,
    attack_ticks: u32,
    /// The main hand's item at the last player tick, whose attack damage and
    /// speed modifiers apply (`LivingEntity.detectEquipmentUpdates`): a hotbar
    /// swap just before an attack keeps them, with the new item's enchantments
    /// (attribute swapping).
    attack_item: Option<Arc<str>>,
    /// Mobs a player hurt (`LivingEntity.lastHurtByPlayer`), by player UUID.
    last_hurt_by_player: HashMap<u32, combat::PlayerCredit>,
    /// Leashed mobs a player killed, with the killer's UUID: the lead drops
    /// on the corpse's next tick (`Leashable.tickLeash`), for the mods. With
    /// the corpse's dimension: each level's tick prunes only its own.
    lead_killers: HashMap<u32, (u128, Dim)>,
    /// Animals a player fed into love (`Animal.loveCause`), with the guest it was by
    /// connection (None: the host). Breeding credits them, wherever they are.
    love_cause: HashMap<u32, Option<u32>>,
    /// Mobs whose death (`LivingEntity.die`) has been handled.
    /// Looked up per mob each tick: a fast hasher (never iterated).
    died: sunrise_client::fast_hash::FxHashSet<u32>,
    /// Client-ticked block-entity animation state (`game/blocks/block_entity_anims.rs`).
    be_anims: sunrise_client::block_entity_anim::BlockEntityAnims,
    /// Item being used (eaten): hotbar slot and ticks used.
    using: Option<(usize, u32)>,
    /// `getUseDuration` at the use's start, as a mod left it (`game/items/item_use.rs`).
    use_duration: i32,
    /// What that use began with (`useItem`) and whether a mod set its ticks left
    /// (`game/items/item_use.rs`).
    use_start: using_item::UseStart,
    /// The crossbow charge's start and middle loading sounds played (`game/items/crossbow.rs`):
    /// each charge plays each once, however far a mod's tick moved the charge at a time.
    crossbow_sounds: [bool; 2],
    /// Camera.oldFovModifier / fovModifier.
    /// `Camera` (eye height and field-of-view easing, view placement).
    view: sunrise_client::camera::Camera,
    /// `FramerateLimiter` and its clock.
    limiter: sunrise_client::window::FramerateLimiter,
    frame_clock: sunrise_client::window::SystemClock,
    /// Hud.scopeScale (spyglass overlay size).
    scope_scale: f32,
    /// Duration of the last frame in ticks (GUI easing).
    frame_ticks: f32,
    title: Option<Title>,
    /// Armor slots: head, chest, legs, feet.
    armor: [Option<ItemStack>; 4],
    /// The off hand (Inventory slot 40, saved as slot 150; menu slot 504 here).
    offhand: Option<ItemStack>,
    /// The body and saddle slots (`EquipmentSlot.BODY`, `SADDLE`; Inventory
    /// slots 41 and 42, saved as 151 and 152). The hands and armor stay in
    /// the fields above. Nothing in the game reads these yet; they are kept
    /// for mods and saves.
    equipment: sunrise_items::equipment::Equipment,
    /// `ItemStack.popTime` of the nine hotbar stacks, then the off hand: set to 5
    /// when a slot gains items, counted down each tick.
    hotbar_pop: [u8; 10],
    /// The hand an item use acts with (`InteractionHand`): the off hand only
    /// for its turn in `use_item` and while its item is being used.
    hand: Hand,
    /// The shield's item cooldown after an axe disabled it (ticks).
    shield_cooldown: u32,
    /// `Player.getCooldowns()`: the `use_cooldown` component's entries, what
    /// `itemDecorations` shades a slot by. A server's players have their own
    /// (`GuestPlay::cooldowns`), swapped in for their turn.
    cooldowns: sunrise_client::cooldowns::Cooldowns,
    /// The source of a command run through /execute (the player otherwise).
    command_source: Option<commands::execute::Source>,
    native_command_filter: bool,
    /// Public chat broadcasts a `chat_recipient` listener's commands start while one runs.
    chat_broadcasts: lan_chat::ChatBroadcasts,
    titles: titles::Titles,
    /// `Hud.toolHighlightTimer`: the selected item name.
    tool_highlight: sunrise_client::hud_overlays::ToolHighlight,
    /// `Hud.autosaveIndicatorValue`.
    saving_indicator: sunrise_client::hud_overlays::SavingIndicator,
    /// This game's world is autosaving this tick (`MinecraftServer.isSaving`).
    autosaving: bool,
    /// `MinecraftServer.ticksUntilAutosave`: the ticks left before the next
    /// autosave, counted down a tick at a time.
    ticks_until_autosave: i64,
    applied_modifiers: Vec<(String, String)>,
    experience: Experience,
    /// `Player.DATA_SCORE_ID`, independent of `totalExperience`.
    player_score: i32,
    orbs: sunrise_client::join_seq::JoinList<ExperienceOrb>,
    xp_delay: u32,
    ordered: ordered_tick::OrderedTicks,
    level_up_sound: sunrise_client::entities::LevelUpSound,
    /// The arm swing (`LivingEntity.SwingState`) and how many swings started.
    swing: sunrise_client::first_person::SwingState,
    swings_started: u32,
    /// Swings of uses whose client predicts them (`SwingSource.PREDICTED`):
    /// a server does not send these back to the player who swung.
    predicted_swings: u32,
    /// The first-person equip heights and the items the hands show (with
    /// the slot each came from).
    heights: sunrise_client::first_person::HandHeights,
    /// The hands' lag behind the view (`LocalPlayer.xBob`, `yBob`); a new
    /// LocalPlayer (joining, respawning, changing dimension) starts at 0.
    sway: sunrise_client::first_person::HandSway,
    visible_hands: [(usize, Option<ItemStack>); 2],
    /// Whether the main and off hand's slots were set to a new stack that
    /// the hand doesn't show yet (`Game::hand_stack_replaced`).
    hands_replaced: [bool; 2],
    /// `Minecraft`'s attack handling, attack clicks since the last tick and
    /// the attack strength tickers the hand reads.
    clicks: sunrise_client::first_person::Clicks<Option<ItemStack>>,
    hand_strength: sunrise_client::attack_strength::AttackStrength,
    settings: Settings,
    /// Video options a mod set for this session only (`set_session_vsync`,
    /// `set_session_clouds`): never in `settings`, so never saved.
    session_video: SessionVideo,
    look: look::LookCache,
    tutorial: tutorial::Tutorial,
    toast_manager: toast_manager::ToastManager,
    /// The open screen's narration (`Screen`'s narration state and timing).
    narration: sunrise_client::screen_narrator::ScreenNarrator,
    /// Where the options screen returns: true = title, false = pause menu.
    options_from_title: bool,
    /// `ClientAvatarState`: walked distance and view bob.
    avatar: sunrise_client::camera::AvatarBob,
    flash: u32,
    /// OptiFine custom sky layers' eased brightness (`custom_sky.rs`).
    custom_sky: custom_sky::CustomSkyView,
    /// `ClientLevel.rainSoundTime`: ticks of rain since the last rain sound.
    rain_sound_time: i32,
    /// Bolts that struck here and sky flashes they made, in total (`status`).
    lightning_seen: [u32; 2],
    /// The head of the bed while sleeping; the visual timer also survives waking.
    sleeping: Option<[i32; 3]>,
    sleep_timer: i32,
    /// The beds players sleep in that this game marked `occupied`, by level
    /// (game/blocks/beds.rs).
    player_beds: Vec<(Dim, [i32; 3])>,
    /// `SleepStatus`'s active and sleeping players at the last announcement.
    sleep_seen: (i64, i64),
    dim: Dim,
    world_root: PathBuf,
    carrier_retention_snapshot: Vec<(u128, u32)>,
    /// Offline-to-account UUID migration: loaded entities retain the old UUID until saved.
    owner_migration: Option<(u128, u128)>,
    /// The world's data packs' content tables, when it has packs (`None`
    /// keeps `Resources.rest`'s own in use).
    world_tables: Option<WorldTables>,
    /// Server-owned loot and item snapshots, independent of client preloads.
    server_loot: sunrise_client::structure_blocks::LootSnapshot,
    server_items: Option<Arc<Items>>,
    /// The world is superflat (`ServerLevel.isFlat`, for every dimension).
    world_flat: bool,
    /// The world is the Debug preset (`Level.isDebug`, for every dimension): its blocks
    /// cannot be changed and nothing ticks.
    world_debug: bool,
    /// A server's players on their way to another level, each waiting for
    /// the destination to load around them.
    server_trips: Vec<server_travel::Trip>,
    /// Players a command sends to another level, once it has run.
    level_teleports: Vec<server_travel::LevelTeleport>,
    /// Teleports a command made inside a level, for `native_player_teleported`
    /// once the player's move has reached their client (the put-back at the end of `guest_turn`).
    teleport_notes: Vec<server_travel::TeleportNote>,
    /// A server's levels other than the one in place (game/world/levels.rs).
    levels: levels::Levels,
    /// Which pass of a server tick runs (`tick_server`): the first one reads
    /// the network, the last one ends the players' tick.
    level_pass: levels::Pass,
    /// Nether portals: the player's, items' and mobs' (game/blocks/nether_portal.rs).
    portals: nether_portal::Portals,
    /// The respawn point when it is not the world spawn or a bed: a respawn
    /// anchor (`game/blocks/respawn_anchor.rs`).
    respawn_config: Option<respawn_anchor::RespawnConfig>,
    /// The next enter_dimension call comes from an End portal.
    end_portal_travel: bool,
    /// The open WinScreen (game/interface/win.rs).
    win: Option<win::Win>,
    demo_intro: Option<demo_intro::DemoIntro>,
    end: end::EndState,
    block_ticks: Ticks,
    tick_scratch: TickScratch,
    tick_chunks: sunrise_client::tick_chunk_membership::TickChunkMembership,
    tickets: sunrise_client::ticket_levels::ChunkTickets,
    random_tick_sections: sunrise_client::random_tick_sections::RandomTickSections,
    level_events: Vec<LevelEvent>,
    /// Falling models by state and selected-model address list; cleared on pack/atlas replacement.
    falling_vertices: RefCell<HashMap<BlockStateId, Vec<(Vec<usize>, Vec<Vertex>)>>>,
    falling: sunrise_client::join_seq::JoinList<FallingBlock, false>,
    /// Blocks the server shows for a moment (`ClientboundAddTransientBlockPacket`).
    transient_blocks: transient_blocks::TransientBlocks,
    paintings: sunrise_client::join_seq::JoinList<sunrise_client::entities::Painting>,
    painting_variants: paintings::PaintingVariants,
    item_frames: sunrise_client::join_seq::JoinList<sunrise_client::entities::ItemFrame>,
    frame_model: frames::FrameModel,
    glow_frame_model: frames::FrameModel,
    /// The `map=true` frame variants.
    map_frame_model: frames::FrameModel,
    glow_map_frame_model: frames::FrameModel,
    armor_stands: sunrise_client::join_seq::JoinList<sunrise_client::entities::ArmorStand>,
    /// Worn armor as of the last equipment check (frost walker).
    location_armor: [Option<ItemStack>; 4],
    /// `Cushion` decoration entities (game/entities/cushions.rs).
    cushions: sunrise_client::join_seq::JoinList<sunrise_client::entities::Cushion>,
    /// Entities of types Sunrise does not simulate.
    others: sunrise_client::join_seq::ResidentList<commands::OtherEntity>,
    armor_stand_assets: armor_stands::ArmorStandAssets,
    client_item_extension_owners: client_item_extensions::ItemExtensionOwners,
    /// `WaypointStyleManager`.
    waypoint_styles: sunrise_client::locator_bar::WaypointStyles,
    /// `ClientWaypointManager`: the locator bar's waypoints.
    waypoints: sunrise_client::locator_bar::ClientWaypoints,
    /// The camera the last frame drew from (position, yaw, pitch, fov), for the locator bar.
    locator_camera: Option<([f64; 3], f32, f32, f32)>,
    native_camera_frame: Option<mod_api::Camera>,
    /// The camera entity's interpolated eye position for the current world frame, or `None`
    /// when there is no camera entity (title screen, scenes).
    native_camera_entity_eye: Option<[f64; 3]>,
    /// The yaw, pitch and roll a native mod's or spectated camera drew the last frame from,
    /// none when it drew from the local player's vanilla view.
    posed_rotation: Option<(f32, f32, f32)>,
    /// `LocalPlayer.postEffects`: the chains the server has the screen run
    /// (`ClientboundPostEffectsPacket`), in order.
    post_effects: Vec<String>,
    /// Reused per-frame `requestedPostEffects` storage.
    requested_post_effects_buf: Vec<String>,
    /// The game time (ticks and partial) of the last frame, for `PolyDeltaTime`.
    polytone_post_ticks: f64,
    /// The last expression context built (what the packs' expressions read), kept across frames
    /// so its strings are rewritten in place instead of allocated each frame.
    expression_frame: sunrise_client::environment::Frame,
    /// `GameRenderer.spectatedEntityPostEffect` (`checkEntityPostEffect`).
    spectated_post_effect: Option<&'static str>,
    /// `GameRenderer.spectatedEntityEffectActive` (`toggleSpectatorPostEffect`).
    spectated_post_effect_active: bool,
    /// `LocalPlayer.experienceDisplayStartTick`: (last progress, seen a change, tick).
    experience_display: (f32, bool, u64),
    boats: sunrise_client::join_seq::JoinList<boats::Boat, false>,
    /// The boats a mob's lead can hang on, for this tick's mob ticks.
    leash_boats: Vec<(u32, sunrise_client::mobs::leash::HolderState)>,
    boat_models: boats::BoatModels,
    /// EndCrystalModel's baked layer.
    crystal_model: Option<sunrise_client::mobs::Part>,
    /// CushionModel's baked layer.
    cushion_model: Option<sunrise_client::mobs::Part>,
    /// Models drawn only once such an entity shows up (lead knots, wind charges...).
    entity_models: EntityModels,
    carts: sunrise_client::join_seq::JoinList<minecarts::Minecart, false>,
    /// The boats and minecarts `native_vehicle_moved` follows (lan_vehicles.rs).
    watched_vehicles: Vec<lan_vehicles::WatchedVehicle>,
    /// Songs playing from jukeboxes (tracked sound handles).
    jukebox_sounds: HashMap<(i32, i32, i32), u64>,
    bolts: sunrise_client::join_seq::ResidentList<lightning::Bolt>,
    maps: maps::Maps,
    /// The ender dragon fight (game/mobs/dragon.rs).
    fight: dragon::Fight,
    dragon: native_entity_refs::dragon_parts_table::DragonSlot,
    crystals: sunrise_client::join_seq::JoinList<dragon::Crystal>,
    breath_clouds: sunrise_client::join_seq::ResidentList<dragon::Breath>,
    dragon_model: Option<sunrise_client::mobs::Part>,
    /// Map textures waiting for the next HUD batch to upload them.
    map_patches: Vec<sunrise_client::font::Patch>,
    name_tag_layouts: names::NameTagLayouts,
    text_displays: indexmap::IndexMap<i32, text_displays::RemoteTextDisplay>,
    see_through_collector: text_displays::SeeThroughCollector,
    translucent_display_collector: text_displays::TranslucentDisplayCollector,
    display_visible_scratch: Vec<(i32, f64)>,
    block_display_parts_scratch: Vec<u32>,
    display_inline_profiles_scratch: Vec<std::sync::Arc<serde_json::Value>>,
    interactions: std::collections::HashMap<i32, interactions::RemoteInteraction>,
    /// The fence knots a server tracks to this client, by entity id, at their
    /// block (`LeashFenceKnotEntity`).
    remote_knots: HashMap<u32, [i32; 3]>,
    /// The entity ids a hosted server gives the knots its mobs hang on, by
    /// block, while a lead is tied there.
    knot_ids: HashMap<[i32; 3], u32>,
    /// Fence posts whose natural knot was killed and whose leads drop next tick.
    broken_knots: Vec<[i32; 3]>,
    text_display_vertices: usize,
    /// Player skin downloads, and which skin each atlas skin slot holds
    /// (least recently shown first).
    skin_downloads: RefCell<sunrise_client::skins::Downloader>,
    /// `ProfileResolver.Cached`: `Dynamic` profiles (name- or id-only heads
    /// and mannequins) resolve through the services in the background.
    skin_profiles: RefCell<sunrise_client::skins::Resolver>,
    skin_slots: sunrise_client::skins::SkinSlots,
    /// What `apply_player_skin_override` resolved for the selected dynamic texture.
    player_skin_slot: Option<native_hooks::resolve_player_skin::PlayerSkinSlot>,
    /// Runtime trim slots and the last completed preparation for this atlas.
    trim_slots: sunrise_client::trims::TrimSlots,
    cart_models: minecarts::CartModels,
    /// The boat the player rides.
    riding: Option<u32>,
    /// This window's player's place in `riding`'s passengers, as `Guest.vehicle_order` is a guest's.
    riding_order: i64,
    /// Vehicles that gained a rider this tick whose `START_RIDING_TRIGGER` was
    /// not fired yet (`game/entities/boarding.rs`), for a guest's turn.
    started_riding: Vec<u32>,
    /// A rotation-only write (`/rotate`, `Entity.forceSetRotation`) to the player of this
    /// turn: the put-back at the end of `guest_turn` sends it as `ClientboundPlayerRotationPacket`, not a teleport.
    rotation_forced: bool,
    /// The mount the last `position_rider` put the player on, to tell the tick
    /// a seat is taken (`Entity.moveTo`, no camera sweep) from the ones that
    /// follow it (`positionRider`, interpolated as usual).
    posed_rider: Option<u32>,
    /// The vehicle the player's file names, waiting for it to be in the world
    /// (`ServerPlayer.loadAndSpawnParentVehicle`).
    pending_vehicle: Option<[i32; 4]>,
    /// The ender inventory (27 slots) and the ender chest showing it.
    ender_items: Vec<Option<ItemStack>>,
    ender_open: Option<(i32, i32, i32)>,
    /// Block entity data of broken blocks waiting for their dropped item.
    pending: spill::Pending,
    /// Item entities spawned and block entities removed while the block
    /// behaviour ran (spilled contents, popped resources).
    spill: spill::Spill,
    /// The recipe book of the open screen (game/items/recipe_book.rs).
    recipe_ui: recipe_book::RecipeUi,
    /// The horse inventory (game/interface/mount_menu.rs).
    mount_menu: mount_menu::MountMenu,
    /// The player's fishing hook.
    hook: sunrise_client::join_seq::ResidentSlot<fishing::Hook>,
    /// The other players' hooks a server tracks to this client.
    remote_hooks: Vec<fishing::RemoteHook>,
    tridents: sunrise_client::join_seq::JoinList<trident::ThrownTrident>,
    rockets: sunrise_client::join_seq::JoinList<fireworks::Rocket>,
    /// Ominous trial spawners' item spawners.
    ominous_spawners:
        sunrise_client::join_seq::ResidentList<ominous_item_spawner::OminousItemSpawner>,
    creaking: creaking::CreakingState,
    /// Monster spawners' display mobs (`game/mobs/spawner.rs`).
    spawners: spawner::SpawnerState,
    /// The active level's stored `ServerChunkCache.spawnEnemies` setting.
    chunk_spawn_enemies: bool,
    /// Reused candidate buffers for natural spawning.
    spawn_scratch: natural_spawning::SpawnScratch,
    trident_model: Option<sunrise_client::mobs::Part>,
    arrow_model: Option<sunrise_client::mobs::Part>,
    bee_stinger_model: Option<sunrise_client::mobs::Part>,
    tnts: sunrise_client::join_seq::JoinList<tnt::PrimedTnt>,
    hoppers: HashMap<(i32, i32, i32), machines::HopperState>,
    /// The open `host.transaction`s, innermost last (`transactions.rs`).
    transactions: transactions::OpenTransactions,
    /// The source of the running dispenser behaviour, including across mod callbacks.
    dispense_source: Option<dispenser::DispenseSource>,
    /// Open mod menus whose `menu_slot_changed` dispatch is currently on the call stack.
    slot_changed_active: Vec<slot_changes::SlotChangedTarget>,
    /// Backing blocks whose contents notice is on the call stack, innermost last.
    contents_changing: Vec<(mod_api::LevelRef, BlockPos)>,
    /// The mod screen of a menu open on this client (`native_screen`).
    mod_screen: Option<native_hooks::OpenNativeScreen>,
    /// The players with each mod container open, by level (`tick_mod_container_viewers`).
    mod_container_viewers:
        HashMap<(&'static str, (i32, i32, i32)), mod_container_openers::ModContainerOpeners>,
    /// Where item entities are, while the block entities tick (hoppers).
    item_columns: Option<machines::ItemColumns>,
    /// The item columns for one pass of the players' pickups
    /// (`tick_guests`), rebuilt after any write to the items (a pickup, a
    /// command, an explosion): see `item_columns`.
    pickup_columns: Option<machines::StampedItemColumns>,
    /// Where the mobs are, while the block entities tick (potent sulfur gas).
    pass_mobs: Option<potent_sulfur::PassMobs>,
    /// Block-entity positions retained between ticks for client effects.
    spawner_tick_positions: Vec<sunrise_client::block_entities::Pos>,
    trial_spawner_tick_positions: Vec<sunrise_client::block_entities::Pos>,
    /// The `[[native.loop]]` slots' state, per published loop (`published_loops.rs`).
    loop_states: published_loops::LoopStates,
    creative_screen: Creative,
    /// The creative search projection whose stacks the mods' `item_name` parts last named.
    creative_search_named: Option<u64>,
    creative_contents: native_hooks::CreativeCache,
    /// `CreativeModeInventoryScreen.ignoreTextInput`: the chat key opened the
    /// search tab, so the letter it carries is not typed into the box.
    creative_ignore_text: bool,
    control: Option<sunrise_client::control::Server>,
    /// The Agent Control setting the socket was last opened or closed for (`agent_settings`).
    agent_control_on: bool,
    /// When an agent's last command came.
    agent_last: Option<Instant>,
    frame_stats: sunrise_client::control::FrameStats,
    /// This thread's and all threads' allocation counts at the last rendered frame.
    alloc_mark: Option<(
        sunrise_threads::alloc_count::Counts,
        sunrise_threads::alloc_count::Counts,
    )>,
    /// This play session's frame and tick times, for opt-in performance statistics.
    perf: sunrise_client::telemetry::Session,
    /// Control replies waiting for a rendered frame or a benchmark deadline.
    pending_screenshot: Option<std::sync::mpsc::Sender<String>>,
    /// `screenshot-sequence` while it runs.
    screenshot_sequence: Option<screenshot_sequence::Sequence>,
    /// `render-capture DIR`: replied once the renderer wrote the scene.
    pending_render_capture: Option<std::sync::mpsc::Sender<String>>,
    pending_bench: Option<(Instant, std::sync::mpsc::Sender<String>, bool)>,
    /// Settings and HUD state: `game/interface/extras.rs`.
    extras: extras::ExtrasState,
    /// The PvP & HUD options' state (`game/entities/pvp.rs`).
    pvp: pvp::PvpState,
    /// Sunrise mods (`game/modding/mods/mod.rs`), none running unless turned on.
    mods: mods::ModsState,
    /// Live recipe and known-display tables for typed mod reads.
    recipe_tables: native_recipes::RecipeTables,
    /// The item panel beside container screens (`game/modding/native_panel.rs`).
    panel: native_panel::Panel,
    /// Sunrise's own shader pack, while one is picked (`game/modding/native_shaders.rs`).
    native_shaders: native_shaders::NativeShaders,
    /// Main-hand input captured by a client mod, independent of living-item use.
    client_item_use: Option<native_claimed_items::ActiveClientItemUse>,
    /// Native client mods' key mappings (game/modding/native_key_mappings.rs).
    mod_keys: sunrise_client::keymapping::ModKeys,
    /// The native lifecycle bookkeeping.
    native_lifecycle: native_lifecycle::NativeLifecycle,
    resident_entities: resident_state::ResidentEntities,
    /// Entities whose copies are held out of storage; their handles refuse until stored back.
    held_out: Vec<u32>,
    /// The game's `config/` folder beside `options.txt` (or `server.properties`
    /// on a dedicated server), lent by `Host::config_dir`.
    config_root: std::sync::Arc<std::path::Path>,
    /// Native mods' config files this game loaded (game/modding/native_configs.rs).
    native_configs: native_configs::NativeConfigs,
    /// Private custom-book connection and placement state.
    native_net: native_net::NativeNet,
    /// The mod whose own screen is open (`Screen::Mod`).
    native_mod_screen: Option<String>,
    /// The open mod screen came from the Mods screen's Config (`IConfigScreenFactory`'s
    /// `parent`): closing it returns to Options > Mods.
    native_mod_screen_parent: bool,
    /// Meshed sections waiting for an upload budget (Sunrise settings).
    pending_uploads: Vec<sunrise_client::streaming::MeshUpload>,
    /// Empty between updates. Reuses storage for merging two sorted runs.
    pending_uploads_merge: Vec<sunrise_client::streaming::MeshUpload>,
    /// The camera section (column, height) `pending_uploads` is sorted for, farthest first:
    /// while it stays, the list needs only its new meshes merged in, not a sort every frame.
    pending_uploads_order: Option<((i32, i32), i32)>,
    upload_pace: upload_pace::UploadPace,
    /// This frame's view frustum, for culling entities and block entities.
    view_frustum: Option<sunrise_client::gfx::JoinedFrusta>,
    occlusion: entity_culling::EntityOcclusion,
    async_particle_culler: RefCell<particle_culling::ParticleAsyncState>,
    /// A scripted flight from the control socket (`fly`).
    flight: Option<control::Flight>,
    /// The running agent action from the control socket (game/runtime/agent/mod.rs).
    agent: agent::Agent,
    /// Widget commands waiting for the frame after a click (game/runtime/ui_control.rs).
    ui_wait: ui_control::UiWait,
    /// What happened, for `events` and `wait-for` (game/runtime/events.rs).
    events: events::Events,
    /// Who caused each input event: a person, an agent or the game (game/runtime/input_trace.rs).
    input_trace: input_trace::InputTrace,
    /// A repro recording, export or replay (game/runtime/repro.rs).
    repro: repro::Repro,
    /// The health history and client captures (game/profile_capture.rs).
    profile: profile_capture::Profile,
    /// `SUNRISE_TICK_SCRIPT`'s scenario (game/world/tick_script.rs).
    tick_script: tick_script::TickScript,
    /// The overworld's last `sky_darken` and its inputs (time, rain, thunder).
    sky_darken_memo: std::cell::Cell<Option<((Dim, i64, u32, u32), i32)>>,
    /// The last water plane `water_plane` found, kept while none is in reach.
    water_plane_memo: std::cell::Cell<Option<f32>>,
    /// The camera's block and the second `water_plane` last scanned at.
    water_plane_scan: std::cell::Cell<Option<([i32; 3], i64)>>,
    /// What the boats' and minecarts' ticks know about the mobs: their box reach
    /// (`vehicle_mob_reach`) and a grid of their boxes (`mobs_meeting`).
    vehicle_mobs: boat_passengers::VehicleMobs,
    persist: persist::EntityMemory,
    /// Missing profession IDs already reported for this world session.
    missing_professions: HashSet<String>,
    entity_attachments: native_colliders::EntityMaps,
    /// What changed in entities' colliders this tick, for the players who track them.
    collider_changes: Vec<native_colliders::ColliderChange>,
    collider_placements: native_colliders::Placements,
    collider_histories: native_colliders::Histories,
    entity_initialization: entity_initialization::EntityInitialization,
    /// Typed per-level membership follows dimensions, not the temporary world swap.
    attachment_indexes: attachment_index::Indexes,
    /// Connected to a server: the world comes from the network (game/network/remote/mod.rs).
    remote: Option<remote::Remote>,
    /// Client work survives a disconnect; packets and credits stay with their physical listener.
    client_tasks: sunrise_client::net::ClientTasks,
    pending_client_tasks: std::collections::VecDeque<sunrise_client::net::ClientTask>,
    /// The current connection came from Realms (retained for reconnect).
    realm_session: bool,
    /// The server this window connected to last over the network: kept while something may
    /// connect there again (game/network/remote/connection.rs).
    last_connection: Option<sunrise_hooks::connection::Server>,
    /// A connection over the network is open: started, not yet closed (one `Hooks::left` or
    /// `Hooks::ended` each).
    connection_open: bool,
    /// The crates' answer to the last connection's end, until the Disconnected screen shows it.
    pending_retry: Option<sunrise_hooks::connection::Retry>,
    /// A connection the Disconnected screen makes on its next frame, as that answer asked.
    retry_due: Option<sunrise_hooks::connection::Server>,
    /// The current connection came from the LAN list (`ServerData.isLan`).
    lan_session: bool,
    /// What the window's title last showed: its session part and the language (window_title.rs).
    window_title: Option<(Option<&'static str>, u64)>,
    /// Replay playback (game/replay.rs), and Sunrise Studio's editor on it (game/studio.rs).
    pub(super) replay: replay::Replay,
    pub(super) studio: studio::Studio,
    /// This world is open to the network (game/lan.rs).
    lan: Option<lan::LanHost>,
    /// Single player's server thread, which `remote` is connected to.
    integrated: Option<integrated::Integrated>,
    /// The shapes of the open world's levels, from its dimension types.
    level_shapes: sunrise_client::chunk::LevelShapes,
    /// This game is that server thread's (game/world/integrated.rs).
    serving: Option<integrated::Serving>,
    /// `IntegratedServer`'s single-player profile, fixed when the server thread is created,
    /// before its startup hooks (game/modding/native_singleplayer_owner.rs). None elsewhere.
    singleplayer_profile: Option<native_singleplayer_owner::SingleplayerProfile>,
    /// The open world is shared behind an invite code (game/network/friends.rs).
    #[cfg(feature = "friends")]
    friends: Option<friends::Sharing>,
    /// The Social Interactions screen and the players whose chat is hidden (game/network/social.rs).
    social: social::Social,
    world_options: Option<lan_screen::WorldOptions>,
    /// Chat Settings opened from RestrictionsScreen: Done returns to World Options.
    options_to_world_options: bool,
    command_block_screen: Option<command_blocks::CommandBlockScreen>,
    block_editor: Option<block_editor::BlockEditScreen>,
    /// `StructureTemplateManager.structureRepository`: the templates in memory
    /// by id, a miss included (game/administration/structure_blocks.rs).
    structure_templates: HashMap<String, Option<structure_blocks::TemplateEntry>>,
    structure_spawns: sunrise_client::mobs::spawn_selection::StructureSpawns,
    /// `GameTestBlockHighlightRenderer.markers`: block, text, removal time.
    test_highlights: HashMap<(i32, i32, i32), (String, std::time::Instant)>,
    /// `GameTestTicker.SINGLETON`: the game tests running on this server.
    game_tests: game_tests::GameTests,
    /// KeyboardHandler's debug key state (game/interface/debug_keys.rs), the physical
    /// keys held, and the debug renderers F3+B and F3+G switch on.
    debug_keys: sunrise_client::debug_keys::DebugKeys,
    /// The F3 overlay's charts (F3+2, F3+3).
    debug_charts: sunrise_client::debug_chart::DebugCharts,
    /// Startup declarations shared by F3 options, the overlay and renderer selection.
    registered_debug_entries: std::sync::Arc<[sunrise_client::debug_entries::RegisteredDebugEntry]>,
    /// `Minecraft.debugEntries`: what the overlay and the debug renderers
    /// show (`debug-profile.json`, F3+F6).
    debug_entries: sunrise_client::debug_entries::DebugEntries,
    /// `DebugOptionsScreen` while it is open (game/interface/debug_options.rs).
    debug_options: Option<debug_options::DebugOptionsUi>,
    /// `DebugEntryMemory`'s readings (game/interface/debug_lines.rs).
    memory_meter: std::cell::RefCell<sunrise_client::debug_overlay::MemoryMeter>,
    /// The F3 overlay's vertices between rebuilds (`draw_debug_overlay_under`).
    debug_paint: RefCell<sunrise_client::debug_overlay::PaintCache>,
    /// The F3 overlay's last reading of the refresh rate, and when it was taken.
    debug_refresh: std::cell::Cell<Option<(Instant, Option<u32>)>>,
    /// `Minecraft.gpuUtilization`: the last timed frame's GPU time over its
    /// CPU time, in percent.
    gpu_utilization: f64,
    /// GPU Culling's Auto mode: what it has decided from the frames' CPU and GPU times.
    gpu_culling_auto: sunrise_client::autotune::GpuCullingAuto,
    /// Dynamic Resolution: the scale it has the world at (`frame_autotune`).
    dynamic_resolution: sunrise_client::dynamic_resolution::DynamicResolution,
    /// The mobs `mob_run_geometry` drew last frame (`lastEntityRenderStateCount`).
    rendered_mobs: std::cell::Cell<usize>,
    /// Whether a native mod requested background blur for this frame (`apply_screen_blur`).
    screen_blur_requested: std::cell::Cell<bool>,
    /// `ServerChunkCache.lastSpawnState` of each level: its spawnable chunks
    /// and each `MobCategory`'s count.
    spawn_counts: HashMap<Dim, (i32, [i32; 8])>,
    /// The natural spawning policy mods set on each level (`LevelRef::set_spawn_limits`,
    /// `set_spawn_flags`); a level with none spawns as vanilla. Kept while the world is open.
    spawn_policies: HashMap<mod_api::DimensionKey, natural_spawning::SpawnPolicy>,
    /// The passages mods registered in each level, and a client's copy (`native_passages.rs`).
    passages: native_passages::Passages,
    /// Non-blocking server chunk load/generation requests (`native_chunk_requests.rs`).
    async_chunk_requests: native_chunk_requests::AsyncChunkRequests,
    /// Open-state flags for `SavedChunkReader` handles per dimension (`native_saved_chunks.rs`).
    saved_chunk_readers: std::cell::RefCell<HashMap<Dim, Arc<std::sync::atomic::AtomicBool>>>,
    /// Open detached chunk generators handed out by `LevelRef::detached_generator`.
    detached_generators: native_detached_generation::DetachedGenerators,
    /// PauseScreen.showPauseMenu (false after F3+Escape).
    pause_menu: bool,
    pause_focus: Option<u8>,
    death_focus: Option<u8>,
    /// The screen ([`Game::key_screen`]) the menu focus belongs to. Another
    /// screen in front drops it, as `Gui.setScreen` drops the focus of the
    /// screen it leaves (`Screen.clearFocus`).
    focus_screen: u64,
    /// MultiPlayerGameMode.previousLocalPlayerMode (F3+N goes back to it).
    previous_mode: Option<sunrise_client::command::args::GameMode>,
    gamemode_switcher: Option<sunrise_client::game_mode_switcher::GameModeSwitcher>,
    /// The system clipboard (`KeyboardHandler.clipboardManager`).
    clipboard: sunrise_client::clipboard::SystemClipboard,
    /// The game is handling a key, mouse or typed-text event, from the window or the
    /// control socket: the only time a mod reaches the clipboard (`with_user_input`).
    user_input: std::cell::Cell<bool>,
    /// Select All in the append-only text fields.
    text_selection: sunrise_client::clipboard::AppendSelection,
    /// The client loop consumes a mod's quit request outside the calling hook.
    client_quit: std::cell::Cell<saving::ClientQuit>,
    /// The world was saved for the application ending (`save_for_exit`).
    saved_for_exit: bool,
    pending_save: Option<saving::ClosingSave>,
    /// Scheduled end of the server tick (`MinecraftServer.haveTime`).
    server_tick_deadline: Option<Instant>,
    /// The last world frame reached the swap chain. A window the compositor
    /// stops showing gets no frames, and the event loop must not spin on it.
    frame_drawn: bool,
    /// The world list's icon in progress (`GameRenderer.tryTakeScreenshotIfNeeded`):
    /// `None` without a local world, or once the save has its icon.png.
    world_icon: Option<WorldIcon>,
    /// The block a guest is acting on (game/network/lan_actions/interaction.rs).
    forced_hit: Option<Hit>,
    /// Non-window simulation's tick-local block ray. Windows use client_pick.
    tick_hit: std::cell::Cell<Option<Option<Hit>>>,
    /// Non-window simulation's tick-local mob ray, including the held attack range.
    tick_mob: Option<Option<(usize, f64, [f64; 3])>>,
    /// `Player.entityInteractionRange` and `blockInteractionRange`, refreshed
    /// each tick from the attribute (a server's reach modifier, creative's
    /// +2): the pick helpers' `&self` reach.
    entity_reach: f64,
    block_reach: f64,
    client_pick: native_hooks::ClientPick,
    /// The part of a right click a guest's packet asks for (game/network/lan_actions/interaction.rs).
    guest_use: Option<GuestUse>,
    /// The last `use_hand` that ended the click did so with `FAIL`, which
    /// does not consume the action (handleUseItemOn's ANY_BLOCK_USE).
    use_failed: bool,
    block_use_original: Vec<(Hand, Option<ItemStack>)>,
    /// Level events of this tick for the guests, with their excluded source.
    lan_events: Vec<QueuedLevelEvent>,
    /// Jukebox level events 1010 (song name) and 1011 (`None`) for the guests;
    /// the song's network id is only known once the hub is in hand.
    lan_songs: Vec<([i32; 3], Option<String>)>,
    lan_cushion_breaks: Vec<(sunrise_world::registry::BlockStateId, [f64; 3])>,
    /// Block events (position, b0, b1, state) and entity events (id, event) for the guests.
    lan_block_events: Vec<([i32; 3], i32, i32, u32)>,
    lan_entity_events: Vec<(i32, i8)>,
    /// Successful crits (attacking guest, hit entity), queued while the hub is taken.
    lan_critical_hits: Vec<(Option<u32>, i32, bool)>,
    /// Spear stabs that landed (the stabbing guest), queued while the hub is
    /// taken: `broadcastEntityEvent(player, 2)`.
    lan_kinetic_hits: Vec<Option<u32>>,
    /// `LivingEntity.recentKineticEnemies`: when each player (a guest, or the
    /// window's own) last stabbed each entity this use.
    stabbed: HashMap<(Option<u32>, u32), i64>,
    /// The other players the acting player's spear can reach this turn,
    /// taken from the hub before it (`spear_players`).
    stab_players: Vec<spear_players::StabPlayer>,
    /// Spear stabs on players worked out in a guest's turn, landed once the
    /// hub is back (`land_player_stabs`).
    player_stabs: Vec<spear_players::StabBatch>,
    /// The movement a guest's client reported this tick
    /// (`ServerPlayer.getKnownMovement`), for their turn.
    known_movement: Vec3,
    /// `LivingEntity.lastKineticHitFeedbackTime` of the entities that felt a
    /// spear's hit (`spear::OWN` is this window's player), in game time.
    kinetic_feedback: HashMap<i32, i64>,
    /// Global level events for every guest (`ServerLevel.globalLevelEvent`), with their level.
    lan_global_events: Vec<(&'static str, i32, [i32; 3], i32)>,
    /// `ContainerOpenersCounter`: how many players (the host and the guests)
    /// have each chest open.
    lid_openers: HashMap<(i32, i32, i32), i32>,
    lan_acting: Option<u32>,
    /// A mod asked for the command trees again (`native_refresh_commands`):
    /// the next server tick asks every mod for its commands and resends them.
    native_commands: native_server::NativeCommandState,
    /// Client-side native mod commands bound to the current client dispatcher generation.
    pub(in crate::game) native_client_commands: native_server::NativeClientCommandState,
    /// The abilities of the server player whose turn it is (`as_guest`): their record's
    /// copy, which the turn reads and writes and the put-back at the end of `guest_turn` stores. None outside a turn.
    acting_abilities: Option<sunrise_client::lan::abilities::PlayerAbilities>,
    /// Other players in the current guest turn, including borrowed ancestor bodies.
    lan_building_boxes: building_players::BuildingPlayers,
    /// A guest's `/save-all`: saved once their turn ends (`run_guest_command`),
    /// when their own state is back in their record.
    save_after_guest_turn: bool,
    /// Guests whose records are displaced by nested player swaps, outer first.
    lan_guest_swaps: Vec<u32>,
    /// Bodies and their statistics displaced by guest turns, outer first.
    lan_displaced_players: Vec<lan_actions::DisplacedPlayer>,
    /// Connections whose turn a panic interrupted this tick: the turn helpers record the
    /// player before re-raising, the end-of-turn path records it before its callbacks run and
    /// clears it after a clean put-back. Cleared at the start of every server tick; the crash
    /// save leaves these players' files as last saved (`server_lifecycle`).
    interrupted_turns: Vec<u32>,
    /// What teardown decided about the interrupted turns, for the writes after it.
    crash_save: server_lifecycle::CrashSave,
    /// Ender chests a mod opened for a guest inside that guest's turn
    /// (`player_open_ender_chest`), by connection, with their titles: opened as the turn
    /// ends, once the guest's own ender chest is back in their record.
    lan_ender_menus: Vec<(u32, sunrise_core::nbt::Tag)>,
    /// Live attachments and original snapshots retained during player replacement.
    player_state: original_player::PlayerState,
    /// Active player breaks waiting on `break_block` listeners.
    active_player_breaks: Vec<ActivePlayerBreak>,
    /// The name of the guest `lan_acting` acts for (`Player.getName`).
    lan_acting_name: Option<String>,
    /// Their UUID (`Player.getUUID`).
    lan_acting_uuid: Option<u128>,
    /// Where the mods refused to prime TNT: the sound and game event the level
    /// queued right after the prime are dropped (`prime_tnt_event`).
    tnt_refused: Option<BlockPos>,
    /// Their permission level: an op's above 0 (`PlayerList.isOp`).
    lan_acting_permission: Option<u8>,
    /// The spawn protection radius that holds the acting guest back
    /// (`DedicatedServer.isUnderSpawnProtection` before the position test):
    /// None for an operator, without operators, or off a dedicated server.
    lan_acting_spawn_protection: Option<i32>,
    /// Their entity id, which the hub (often taken out of `lan` while they
    /// act) would otherwise have to be asked for.
    lan_acting_entity: Option<u32>,
    /// Whether the player currently swapped into the game is connectionless.
    lan_acting_fake: bool,
    /// A fake player's `perform_action` in progress, and what it refused.
    fake_action: std::sync::Arc<native_server::FakeAction>,
    /// Command feedback to forward to a guest.
    chat_capture: Option<Vec<sunrise_client::text::Text>>,
    /// Packets for the guest acting now, sent once its action returns.
    guest_packets: Vec<sunrise_protocol::packets::play::Clientbound>,
    /// The acting guest's cooldowns, as (item, ticks): their packet needs the
    /// hub's item registry, which the guest's turn does not hold.
    guest_cooldowns: Vec<(Arc<str>, i32)>,
    /// The acting guest's own entity events (`ServerPlayer.connection.send`),
    /// sent with their entity id, which the guest's turn does not hold.
    guest_entity_events: Vec<i8>,
    /// The acting guest's equipment breaks (`onEquippedItemBroken`'s entity
    /// events), broadcast with their entity id once their turn ends.
    guest_equipment_breaks: Vec<i8>,
    /// Block sounds queued by the acting guest, excluding that guest.
    guest_block_sounds: Vec<(String, [f64; 3], f32, f32)>,
    /// `Player.playSound` of the acting guest (event, position, volume,
    /// pitch): `level.playSound(player, ...)` leaves the guest out, whose
    /// own client plays it.
    guest_player_sounds: Vec<(String, [f64; 3], f32, f32)>,
    /// Sounds bound to an entity this tick, for the players (`game/interface/level_sounds.rs`).
    bound_sounds: Vec<level_sounds::BoundSound>,
    /// `broadcastToAdmins` lines for the other operators that a command queued,
    /// sent once it returns (`send_admin_feedback`), each with the guest whose
    /// command it was (`commandSource() != source`): None for a block, a
    /// minecart, the console or the host.
    admin_feedback: Vec<(sunrise_client::text::Text, Option<u32>)>,
    /// The server console is running a command: whether its feedback goes to
    /// the operators (`DedicatedServer.shouldInformAdmins`,
    /// `broadcast-console-to-ops`).
    console_broadcast: Option<bool>,
    /// The command the console is running came over RCON: its source is named `Rcon`
    /// (`RconConsoleSource.createCommandSourceStack`), not `Server`.
    rcon_source: bool,
    /// A dedicated server's operator, whitelist and ban lists, for `/op`,
    /// `/ban` and friends (`DedicatedPlayerList`).
    server_lists: Option<std::sync::Arc<std::sync::Mutex<dedicated::Lists>>>,
    /// `Services.nameToIdCache`: the server thread's usercache.json.
    user_cache: Option<sunrise_client::server::user_cache::UserCache>,
    /// State of `/spectate`, `/posteffect`, `/debug`, `/waypoint` and friends.
    cmds: commands::extra::CommandExtras,
    /// The retained server reload listeners of the last successful reload.
    reload_resources: mod_api::RetainedListeners,
    ui: chat::UiState,
    combat: combat::CombatTracker,
    /// Each guest's own `CombatTracker`, out of `combat` between turns.
    guest_combat: std::collections::HashMap<u32, combat::CombatTracker>,
    mount: mounts::MountState,
    stats: stats::Stats,
    score: scoreboard::ScoreState,
    fns: functions::FnState,
    bosses: bossbars::BossState,
    /// `Hud.bossOverlay`: the bars this window is showing.
    boss_overlay: sunrise_client::boss_overlay::BossOverlay,
    exec: commands::execute::ExecState,
    rand_sequences: sunrise_client::random_sequences::RandomSequences,
    /// `BrushableBlockEntity`'s brush counts of the blocks being brushed.
    brushing: brush::Brushing,
    border: sunrise_client::worldborder::WorldBorder,
    /// `WorldBorderRenderer` and its render state.
    border_renderer: sunrise_client::border_render::BorderRenderer,
    /// The camera's 16-block section when the wall was last invalidated.
    border_section: Option<[i32; 3]>,
    border_state: sunrise_client::border_render::BorderRenderState,
    /// `Hud.tabList`: the player list's header, footer and heart states.
    tab_list: sunrise_client::tab_list::TabList,
    subtitles: subtitle_hud::SubtitleState,
    adv: advancements::AdvState,
    restore_player_state: bool,
    stats_ui: stats_screen::StatsUi,
    adv_ui: advancements_screen::AdvUi,
    /// The Server Links screens' state (game/network/server_links.rs).
    links_ui: server_links::LinksUi,
    camera_type: sunrise_client::camera::CameraType,
    requested_camera_type: std::cell::Cell<Option<sunrise_client::camera::CameraType>>,
    fangs: fangs::FangState,
    /// The local player as a model (third person, the inventory).
    player_model: Option<Mob>,
}
/// The open world's dimensions past vanilla's three. One per process, as a window
/// and its server thread name the same dimensions.
static EXTRA_DIMS: std::sync::RwLock<Vec<mod_api::DimensionKey>> =
    std::sync::RwLock::new(Vec::new());
/// Custom level keys parsed when the world registers its dimensions, not on each block query.
static EXTRA_LEVELS: std::sync::RwLock<
    Vec<(mod_api::DimensionKey, sunrise_core::dimension::Dimension)>,
> = std::sync::RwLock::new(Vec::new());

/// An active player break waiting on `break_block` listeners. When a nested
/// player break completes removal at the same `(dim, pos)`, `removed_by_nested`
/// is set so the outer break does not apply removal effects again even if a
/// non-air block or fluid remains (such as fewer turtle eggs or water).
struct ActivePlayerBreak {
    dim: Dim,
    pos: [i32; 3],
    removed_by_nested: bool,
}

struct ActiveBreakGuard<'g, 'a> {
    game: &'g mut Game<'a>,
    depth: usize,
}

impl Drop for ActiveBreakGuard<'_, '_> {
    fn drop(&mut self) {
        self.game.active_player_breaks.truncate(self.depth);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Dim {
    Overworld,
    Nether,
    End,
    /// A dimension past vanilla's three, from a data pack or a mod.
    Extra(mod_api::DimensionKey),
}
impl Dim {
    /// The level as block behaviour and the mods name it.
    fn level(self) -> LevelDimension {
        match self {
            Self::Overworld => LevelDimension::Overworld,
            Self::Nether => LevelDimension::Nether,
            Self::End => LevelDimension::End,
            Self::Extra(key) => EXTRA_LEVELS
                .read()
                .unwrap()
                .iter()
                .find_map(|(candidate, level)| (*candidate == key).then_some(*level))
                .unwrap_or_else(|| {
                    LevelDimension::of_level(
                        sunrise_core::LevelKey::parse(self.id())
                            .unwrap_or(sunrise_core::LevelKey::OVERWORLD),
                    )
                }),
        }
    }
    fn id(self) -> &'static str {
        match self {
            Self::Overworld => "minecraft:overworld",
            Self::Nether => "minecraft:the_nether",
            Self::End => "minecraft:the_end",
            Self::Extra(key) => {
                mod_api::__private::dimension_id(key).expect("the game's table made the key")
            }
        }
    }
    fn from_id(id: &str) -> Self {
        match id {
            "minecraft:the_nether" => Self::Nether,
            "minecraft:the_end" => Self::End,
            _ => Self::extra(id).unwrap_or(Self::Overworld),
        }
    }
    /// `Level.getSeaLevel` of the level: the sea level its noise settings give a mod's level, else
    /// the built-in level's.
    fn sea_level(self) -> i32 {
        Self::sea_level_of_id(self.id())
    }
    /// [`Self::sea_level`] of the level `id` names.
    fn sea_level_of_id(id: &str) -> i32 {
        sunrise_core::LevelKey::parse(id).map_or(63, sunrise_world::level::sea_level_of)
    }
    /// The open world's dimension `id` past vanilla's three.
    fn extra(id: &str) -> Option<Self> {
        let extra = EXTRA_DIMS.read().unwrap();
        let key = extra
            .iter()
            .find(|key| mod_api::__private::dimension_id(**key) == Some(id))?;
        Some(Self::Extra(*key))
    }
    fn is_vanilla(id: &str) -> bool {
        matches!(
            id,
            "minecraft:overworld" | "minecraft:the_nether" | "minecraft:the_end"
        )
    }
    /// Whether this is one of vanilla's built-in levels (Overworld, Nether, End).
    fn is_builtin(self) -> bool {
        matches!(self, Self::Overworld | Self::Nether | Self::End)
    }
    /// Whether the open world has the dimension `id`.
    fn exists(id: &str) -> bool {
        Self::is_vanilla(id) || Self::extra(id).is_some()
    }
    /// The open world's dimensions, in the order its server ticks them.
    fn all() -> Vec<Self> {
        let extra = EXTRA_DIMS.read().unwrap();
        [Self::Overworld, Self::Nether, Self::End]
            .into_iter()
            .chain(extra.iter().copied().map(Self::Extra))
            .collect()
    }
    /// The dimensions past vanilla's three of the world being opened.
    fn set_extra<'a>(ids: impl Iterator<Item = &'a str>) {
        let extra: Vec<_> = ids
            .map(|id| {
                (
                    mod_api::__private::dimension_key(id),
                    LevelDimension::of_level(
                        sunrise_core::LevelKey::parse(id)
                            .unwrap_or(sunrise_core::LevelKey::OVERWORLD),
                    ),
                )
            })
            .collect();
        *EXTRA_DIMS.write().unwrap() = extra.iter().map(|(key, _)| *key).collect();
        *EXTRA_LEVELS.write().unwrap() = extra;
    }
    /// Adds the dimension `id` a remote server sent, which no local world told of, so that
    /// [`Self::from_id`] finds it.
    fn add_extra(id: &str) {
        if !Self::exists(id) {
            let key = mod_api::__private::dimension_key(id);
            let level = LevelDimension::of_level(
                sunrise_core::LevelKey::parse(id).unwrap_or(sunrise_core::LevelKey::OVERWORLD),
            );
            EXTRA_DIMS.write().unwrap().push(key);
            EXTRA_LEVELS.write().unwrap().push((key, level));
        }
    }
    /// Drops a remote or runtime dimension that is no longer part of the open world.
    pub(in crate::game) fn remove_extra(id: &str) {
        EXTRA_DIMS
            .write()
            .unwrap()
            .retain(|key| mod_api::__private::dimension_id(*key) != Some(id));
        EXTRA_LEVELS
            .write()
            .unwrap()
            .retain(|(key, _)| mod_api::__private::dimension_id(*key) != Some(id));
    }
    /// The dimension as mods' level handles name it.
    fn key(self) -> mod_api::DimensionKey {
        match self {
            Self::Overworld => mod_api::DimensionKey::OVERWORLD,
            Self::Nether => mod_api::DimensionKey::NETHER,
            Self::End => mod_api::DimensionKey::END,
            Self::Extra(key) => key,
        }
    }
    /// The level's shape in a world of vanilla's dimension types.
    fn shape(self) -> sunrise_client::save::LevelShape {
        use sunrise_client::save::LevelShape;
        match self {
            Self::Overworld => LevelShape::OVERWORLD,
            Self::Nether => LevelShape::NETHER,
            Self::End => LevelShape::END,
            Self::Extra(_) => LevelShape::OVERWORLD,
        }
    }
}
/// `Inventory.add` on a player's 36 slots (hotbar first) and off hand,
/// `max` the stack's own maximum, one slot at a time (`addResource`): onto an
/// equal stack while there is one (`getSlotWithRemainingSpace`), else into an
/// empty slot of the inventory (`getFreeSlot`, never the off hand), until the
/// stack is used up, no slot is left or a slot takes none of it. What does
/// not fit stays in `stack`; true when some of it is left.
fn inventory_add(
    inventory: &mut [Option<ItemStack>],
    offhand: &mut Option<ItemStack>,
    selected: usize,
    max: u32,
    stack: &mut ItemStack,
) -> bool {
    use sunrise_client::menu::{OFFHAND, free_slot, slot_with_remaining_space};
    loop {
        // `Inventory.add`: an empty stack adds nothing (`isEmpty`).
        if stack.count == 0 {
            return false;
        }
        let slot =
            slot_with_remaining_space(&inventory[..36], offhand.as_ref(), selected, stack, max)
                .or_else(|| free_slot(&inventory[..36]));
        #[cfg(test)]
        let slot =
            rule_replacements::add_slot(&inventory[..36], offhand.as_ref(), selected, stack, max)
                .unwrap_or(slot);
        let Some(slot) = slot else {
            return true;
        };
        let cell = if slot == OFFHAND {
            &mut *offhand
        } else {
            &mut inventory[slot]
        };
        let moved = cell
            .as_ref()
            .map_or(max, |cell| max.saturating_sub(cell.count))
            .min(stack.count);
        // `Inventory.add`'s loop ends once an `addResource` moves nothing
        // (`getCount() < lastSize`): a slot with no room for the stack.
        if moved == 0 {
            return true;
        }
        match cell {
            Some(cell) => cell.count += moved,
            None => {
                *cell = Some(ItemStack {
                    count: moved,
                    ..stack.clone()
                })
            }
        }
        stack.count -= moved;
    }
}

fn facing_offset(facing: &str) -> [i32; 3] {
    match facing {
        "north" => [0, 0, -1],
        "south" => [0, 0, 1],
        "west" => [-1, 0, 0],
        "east" => [1, 0, 0],
        "up" => [0, 1, 0],
        _ => [0, -1, 0],
    }
}
/// How far along `dir` (within `reach`) a ray from `eye` enters `b`.
fn ray_box(eye: [f64; 3], dir: [f64; 3], reach: f64, b: Aabb) -> Option<f64> {
    let (min, max) = (b.min_position(), b.max_position());
    let (lo, hi) = ([min.x, min.y, min.z], [max.x, max.y, max.z]);
    let (mut t0, mut t1) = (0.0f64, reach);
    for a in 0..3 {
        if dir[a].abs() < 1e-9 {
            if eye[a] < lo[a] || eye[a] > hi[a] {
                return None;
            }
            continue;
        }
        let (mut ta, mut tb) = ((lo[a] - eye[a]) / dir[a], (hi[a] - eye[a]) / dir[a]);
        if ta > tb {
            std::mem::swap(&mut ta, &mut tb);
        }
        t0 = t0.max(ta);
        t1 = t1.min(tb);
    }
    (t0 <= t1).then_some(t0)
}
/// `Entity.calculateViewVector`, behind `getViewVector`, `getLookAngle` and `GameRenderer.pick`: float radians
/// through `Mth.sin` and `Mth.cos`, vanilla's 65536-entry table. Aims, a deflected projectile's motion, a spear's
/// reach and a placement's cursor all come from it.
fn view_vector(yaw: f32, pitch: f32) -> [f64; 3] {
    let v = sunrise_world::view_direction::view_vector(pitch, yaw);
    [v.x, v.y, v.z]
}
#[cfg(test)]
mod view_vector_tests {
    /// `Entity.calculateViewVector` on HotSpot 25.0.4.1 (x86-64) for angles where double-precision `sin`/`cos`
    /// gave other bits: (pitch, yaw) as float bits, vanilla's x, y and z as double bits.
    #[test]
    fn the_view_vector_has_vanillas_bits() {
        let cases: [(u32, u32, [u64; 3]); 4] = [
            (
                0xc1d5_be2f,
                0x43b0_17dc,
                [
                    0x3fbf_198b_0000_0000,
                    0x3fdc_c507_6000_0000,
                    0x3fec_514d_4000_0000,
                ],
            ),
            (
                0x42b4_0000,
                0x43b4_0001,
                [
                    0x0000_0000_0000_0000,
                    0xbff0_0000_0000_0000,
                    0x3ca1_a626_4000_0000,
                ],
            ),
            (
                0xc2ad_4925,
                0x43a0_0c61,
                [
                    0x3fa3_3b12_a000_0000,
                    0x3fef_f1e7_2000_0000,
                    0x3fa6_fe4a_e000_0000,
                ],
            ),
            (
                0x3b32_7d58,
                0x4ea6_1ddd,
                [
                    0xbfd0_9895_4000_0000,
                    0x8000_0000_0000_0000,
                    0xbfee_e7c7_8000_0000,
                ],
            ),
        ];
        for (pitch, yaw, expected) in cases {
            let v = super::view_vector(f32::from_bits(yaw), f32::from_bits(pitch));
            assert_eq!(
                v.map(f64::to_bits),
                expected,
                "pitch {pitch:#x}, yaw {yaw:#x}"
            );
        }
    }
}
/// `Camera.setRotation` (`rotationYXZ(π - yaw, -pitch, 0)`) for the player's
/// view, turned round in third-person front view (`Camera.setup`).
fn camera_rotation(
    camera_type: sunrise_client::camera::CameraType,
    yaw: f32,
    pitch: f32,
) -> glam::Quat {
    camera_rotation_rolled(camera_type, yaw, pitch, 0.0)
}
/// [`camera_rotation`] with roll in degrees (`ViewportEvent.ComputeCameraAngles`).
fn camera_rotation_rolled(
    camera_type: sunrise_client::camera::CameraType,
    yaw: f32,
    pitch: f32,
    roll: f32,
) -> glam::Quat {
    let (view_yaw, view_pitch, view_roll) =
        if camera_type == sunrise_client::camera::CameraType::ThirdPersonFront {
            (yaw + 180.0, -pitch, -roll)
        } else {
            (yaw, pitch, roll)
        };
    let q = sunrise_client::camera::rotation_with_roll(view_yaw, view_pitch, view_roll);
    glam::Quat::from_xyzw(q.x, q.y, q.z, q.w)
}
/// The world directions of a camera-facing quad's x and y: `PoseStack.rotate` by the camera's
/// orientation, as `FishingHookRenderer`, `DragonFireballRenderer` and
/// `SubmitNodeCollection.submitNameTag` turn theirs. Both stay whole looking straight up or
/// down, where the view vector has no horizontal part.
fn billboard_axes(orientation: glam::Quat) -> [glam::DVec3; 2] {
    [glam::Vec3::X, glam::Vec3::Y].map(|axis| (orientation * axis).as_dvec3())
}
#[cfg(test)]
mod billboard_axes_tests {
    /// `Camera.setRotation(30, -90)` (looking straight up) on HotSpot 25.0.4.1: its orientation
    /// turns x and y to these, as float bits. glam's quaternion is not JOML's to the last bit.
    #[test]
    fn a_billboard_looking_straight_up_takes_the_cameras_axes() {
        let expected: [[u32; 3]; 2] = [
            [0xbf5d_b3d8, 0x0000_0000, 0xbf00_0002],
            [0x3f00_0002, 0xb180_0001, 0xbf5d_b3d7],
        ];
        let axes = super::billboard_axes(super::camera_rotation(
            sunrise_client::camera::CameraType::FirstPerson,
            30.0,
            -90.0,
        ));
        for (axis, expected) in axes.into_iter().zip(expected) {
            let expected = glam::Vec3::from_array(expected.map(f32::from_bits)).as_dvec3();
            assert!(
                axis.abs_diff_eq(expected, 1e-6),
                "{axis} against {expected}"
            );
        }
    }
}
/// What the falling blocks of one frame share: the depth-only view (none under a pack) and the
/// camera's forward axis.
struct FallingSetup<'a> {
    view: Option<&'a sunrise_client::gfx::JoinedFrusta>,
    forward: glam::Vec3,
}

/// What the experience orbs of one frame share: their sprite and the camera's axes.
pub(crate) struct OrbFrame {
    sprite: Option<sunrise_client_render::hud::Rect>,
    forward: glam::Vec3,
    right: glam::Vec3,
    up: glam::Vec3,
}

impl Game<'_> {
    /// `Level.canHaveWeather`, from the level's dimension type (sky light, no
    /// ceiling) and the End, which vanilla names by id. Every rain, thunder
    /// and lightning rule asks this, never the dimension's id.
    fn level_key(&self) -> sunrise_core::LevelKey {
        sunrise_core::LevelKey::builtin_or_overworld(self.dim.id())
    }
    /// The level's dimension type: a mod's or a data pack's own when it has one, else the
    /// built-in level's.
    fn dimension_type(&self) -> &sunrise_world::dimension_type::DimensionFacts {
        // A client of a server has the facts its server sent for the level, which are its own,
        // not the table the server's levels (in this process, for an integrated server) read.
        if let Some(facts) = self.remote.as_ref().and_then(|r| r.level_facts.as_deref()) {
            return facts;
        }
        // Else the level the server named, whether or not the client has the mod that
        // registered it (`self.dim` is then the Overworld).
        let level = self
            .remote
            .as_ref()
            .map(|r| r.level.as_str())
            .filter(|level| !level.is_empty())
            .unwrap_or_else(|| self.dim.id());
        self.dimension_facts_for(
            sunrise_core::LevelKey::parse(level).unwrap_or(sunrise_core::LevelKey::OVERWORLD),
        )
    }
    /// A client only knows its current level's type; other keys use built-in defaults.
    fn dimension_facts_for(
        &self,
        key: sunrise_core::LevelKey,
    ) -> &sunrise_world::dimension_type::DimensionFacts {
        use sunrise_world::dimension_type::DimensionFacts;
        if let Some(remote) = &self.remote {
            if sunrise_core::LevelKey::parse(&remote.level).ok() == Some(key)
                && let Some(facts) = remote.level_facts.as_deref()
            {
                return facts;
            }
            DimensionFacts::of_builtin_level(key)
        } else {
            DimensionFacts::of_level(key)
        }
    }
    fn dimension_type_of(
        &self,
        dim: Dim,
    ) -> &'static sunrise_world::dimension_type::DimensionFacts {
        sunrise_world::dimension_type::DimensionFacts::of_level(
            sunrise_core::LevelKey::parse(dim.id()).unwrap_or(sunrise_core::LevelKey::OVERWORLD),
        )
    }
    /// Sets up the same block-entity scene for both world draw paths.
    fn with_block_entity_scene(
        &self,
        camera: glam::DVec3,
        partial: f32,
        dynamic_visible: &dyn Fn([i32; 3], u32, glam::DVec3, glam::DVec3) -> bool,
        draw: impl FnOnce(&BlockEntityScene<'_>),
    ) {
        let lid = |p: [i32; 3]| self.chest_lid(p, partial);
        let anim = |p: [i32; 3]| self.block_anim(p, partial);
        let substitute = |text: &sunrise_client::text::Text| self.substitute_inline_images(text);
        let inline_sprite = |atlas: &str, sprite: &str| self.gui.atlas.inline_sprite(atlas, sprite);
        use block_entity_render::SignOutline;
        let sign_outline =
            if self.player.is_some() && self.camera_type.is_first_person() && self.scoping() {
                SignOutline::FirstPersonScope
            } else {
                self.camera_subject().map_or(SignOutline::None, |subject| {
                    SignOutline::CameraEntity(glam::DVec3::from_array(subject.camera.position))
                })
            };
        let scene = BlockEntityScene {
            camera,
            sign_outline,
            orientation: self.camera_orientation(),
            anim: &anim,
            texts: None,
            partial,
            game_time: self.world.game_time,
            max_y: self.world.shape.max_y() - 1,
            world: &self.world,
            sprites: &self.gui.atlas.sprites,
            emissive: &self.gui.atlas.emissive,
            translucent_sprites: &self.gui.atlas.translucent,
            atlas_revision: self.gui.atlas.revision,
            nether: self.nether_lighting(),
            lid: &lid,
            sign: &|p| match self.world.block_entities.get(&(p[0], p[1], p[2])) {
                Some(BlockEntity::Sign(sign)) => Some(sign),
                _ => None,
            },
            inline_images: self.inline_images_revision().map(|revision| {
                block_entity_render::SignInlineImages {
                    revision,
                    substitute: &substitute,
                    sprite: &inline_sprite,
                }
            }),
            banner: &|p| self.banner_layers_at(p),
            skull: &|p| self.skull_skin_at(p),
            glyph: &|c| self.gui.atlas.glyph_cell(c),
            glyphs: self.gui.atlas.font.epoch(),
            font: Some(&self.gui.atlas.font),
            dynamic_visible: Some(dynamic_visible),
        };
        draw(&scene);
    }

    fn eye_position(&self) -> Option<[f64; 3]> {
        let p = self.player.as_ref()?;
        Some([
            p.position.x,
            p.position.y + f64::from(p.dimensions.eye_height),
            p.position.z,
        ])
    }
    /// Where targeting starts and points: the eyes along the view. Every
    /// crosshair target (blocks, fluids, entities, mod raycasts) is found
    /// along this ray, so a tracked controller can stand in for it.
    fn aim(&self) -> Option<([f64; 3], [f64; 3])> {
        if let Some(ray) = self.client_pick.ray {
            return Some(ray);
        }
        // Minecraft.pick: from the camera entity's eyes along its view
        // (LivingEntity.getViewYRot: the head's yaw).
        if let Some(e) = self.spectated_camera_snapshot() {
            let [x, y, z] = e.position;
            let eye = [x, y + f64::from(e.eye_height), z];
            return Some((eye, view_vector(e.y_rot, e.x_rot)));
        }
        Some((self.eye_position()?, view_vector(self.yaw, self.pitch)))
    }
    /// `Player.blockInteractionRange`: the block pick's and mining's reach.
    fn reach(&self) -> f64 {
        self.block_reach
    }
    /// Outline shapes that depend on more than the block state, as one box
    /// in block space: `LightBlock` and `ScaffoldingBlock.getShape` are a full
    /// block while the main hand holds the block's own item
    /// (`CollisionContext.isHoldingItem`), and `ShulkerBoxBlock.getShape`
    /// grows with the lid (`ShulkerBoxBlockEntity.getBoundingBox`).
    fn context_outline(
        &self,
        pos: [i32; 3],
        state: BlockStateId,
        player_context: bool,
        anims: &sunrise_client::block_entity_anim::BlockEntityAnims,
    ) -> Option<[f64; 6]> {
        let name = &self.res.facts().get(state).name;
        if matches!(&**name, "minecraft:light" | "minecraft:scaffolding") {
            return (player_context && self.held().is_some_and(|s| *s.item == **name))
                .then_some([0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
        }
        if !name.ends_with("shulker_box") {
            return None;
        }
        let progress = anims.shulkers.get(&pos).map_or(0.0, |s| s.progress);
        let facing = self
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("facing").cloned());
        let d = facing_offset(facing.as_deref().unwrap_or("up"));
        let mut b = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        for a in 0..3 {
            match d[a] {
                1 => b[a + 3] += 0.5 * f64::from(progress),
                -1 => b[a] -= 0.5 * f64::from(progress),
                _ => {}
            }
        }
        Some(b)
    }
    fn outline_context_kind(&self) -> sunrise_world::collision_context::EntityContextKind {
        self.camera_subject().map_or(
            sunrise_world::collision_context::EntityContextKind::Player,
            |c| c.collision_kind,
        )
    }
    /// A block's outline boxes in world space (`getShape` plus the random offset).
    fn outline_boxes(&self, pos: [i32; 3]) -> Vec<[f64; 6]> {
        self.outline_boxes_in_context(&self.world, &self.be_anims, pos, true)
    }
    fn outline_boxes_in_context(
        &self,
        world: &World,
        anims: &sunrise_client::block_entity_anim::BlockEntityAnims,
        pos: [i32; 3],
        player_context: bool,
    ) -> Vec<[f64; 6]> {
        self.outline_boxes_iter(
            world,
            world.block(pos[0], pos[1], pos[2]),
            anims,
            pos,
            player_context,
        )
        .collect()
    }
    /// The declared outline uses vanilla's inside-ray test and interaction override.
    fn clip_declared_outline(
        &self,
        state: BlockStateId,
        pos: [i32; 3],
        origin: [f64; 3],
        dir: [f64; 3],
        reach: f64,
    ) -> Option<Hit> {
        use std::borrow::Cow;
        use sunrise_core::geometry::{Aabb, Vec3};
        let facts = self.res.facts().get(state);
        let cached = self.collision.cached_shapes();
        use native_state_shapes::Outline;
        let (outline, offset) =
            self.selected_outline(&self.world, state, &self.be_anims, pos, true);
        let boxes: Cow<'_, [Aabb]> = match outline {
            Outline::Published(shape) | Outline::Declared(shape, _) => Cow::Borrowed(shape.boxes()),
            Outline::Dynamic(b) => Cow::Owned(vec![Aabb::new(
                Vec3::new(b[0], b[1], b[2]),
                Vec3::new(b[3], b[4], b[5]),
            )]),
            Outline::Default(facts) => Cow::Owned(
                facts
                    .outline
                    .iter()
                    .map(|b| Aabb::new(Vec3::new(b[0], b[1], b[2]), Vec3::new(b[3], b[4], b[5])))
                    .collect(),
            ),
        };
        let offset = Vec3::new(offset[0], offset[1], offset[2]);
        let interaction_offset = cached.shape_offset(
            state,
            sunrise_world::collision::ShapeChannel::Interaction,
            pos,
        );
        let interaction_offset = Vec3::new(
            interaction_offset[0],
            interaction_offset[1],
            interaction_offset[2],
        );
        let interaction: Cow<'_, [Aabb]> = if let Some(shape) = cached.interaction_shape(state) {
            Cow::Borrowed(shape.boxes())
        } else {
            let facing = self
                .pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get("facing"))
                .map(String::as_str);
            Cow::Owned(
                interaction_shapes::boxes(&facts.name, facing)
                    .into_iter()
                    .map(|b| Aabb::new(Vec3::new(b[0], b[1], b[2]), Vec3::new(b[3], b[4], b[5])))
                    .collect(),
            )
        };
        let start = Vec3::new(origin[0], origin[1], origin[2]);
        let end = Vec3::new(
            origin[0] + dir[0] * reach,
            origin[1] + dir[1] * reach,
            origin[2] + dir[2] * reach,
        );
        let hit = sunrise_world::interaction_shapes::clip_with_interaction_override(
            boxes.iter().map(|b| b.move_by(offset)),
            interaction.iter().map(|b| b.move_by(interaction_offset)),
            start,
            end,
            BlockPos::new(pos[0], pos[1], pos[2]),
        )?;
        Some(Hit::from_block_hit(pos, &hit))
    }
    /// Voxel traversal against non-air, non-fluid blocks, clipped against
    /// each block's outline shape (`BlockGetter.clip` with `ClipContext.Block.OUTLINE`).
    fn pick(&self) -> Option<Hit> {
        // A guest of a hosted world acts on the block their client targeted.
        if let Some(hit) = self.forced_hit {
            return Some(hit);
        }
        if self.uses_stored_pick() {
            return match self.crosshair_target() {
                Some(mod_api::CrosshairTarget::Block { pos, hit, .. }) => Some(Hit {
                    pos: [pos.x, pos.y, pos.z],
                    face: hit.face,
                    point: hit.point,
                    inside: hit.inside,
                }),
                _ => None,
            };
        }
        if let Some(hit) = self.tick_hit.get() {
            return hit;
        }
        match self.compute_crosshair_decision() {
            pick_entity::CrosshairDecision::Block(hit) => Some(hit),
            _ => None,
        }
    }
    /// An item-specific POV outline ray at the current rotation and block reach.
    pub(super) fn item_pick(&self) -> Option<Hit> {
        if let Some(hit) = self.forced_hit {
            return Some(hit);
        }
        let (origin, direction) = self.aim()?;
        self.clip_ray(origin, direction, self.block_reach)
    }

    /// The raw outline ray out to the larger interaction range, before range filtering.
    /// Used once by client_pick and by non-window simulation, never by window actions.
    pub(super) fn pick_unfiltered(&self) -> Option<Hit> {
        if let Some(hit) = self.forced_hit {
            return Some(hit);
        }
        let (origin, dir) = self.aim()?;
        self.clip_ray(origin, dir, self.block_reach.max(self.entity_reach))
    }

    /// The first block outline hit within `reach` along the unit `dir`.
    fn clip_ray(&self, origin: [f64; 3], dir: [f64; 3], reach: f64) -> Option<Hit> {
        self.clip_ray_fluids(origin, dir, reach, false)
    }
    /// `LocalPlayer.moveTowardsClosestSpace` at the body's four corners
    /// (aiStep, unless a spectator or a passenger): a corner inside a
    /// suffocating block sets a 0.1 speed towards the nearest side that is
    /// free, on that side's axis; a later corner overwrites an earlier one.
    /// Suffocating is `LocalPlayer.suffocatesAt` (`closest_space_nudge`).
    fn closest_space_nudge(&self) -> [Option<f64>; 2] {
        match self.player.as_ref() {
            Some(p) if !p.spectator && self.riding.is_none() => {
                closest_space_nudge(p, &self.world, self.collision, self.level_suffocation())
            }
            _ => [None; 2],
        }
    }
    /// What the window decides for its player's body every tick: the look
    /// (Player.travel steers sprint-swimming by the pitch), Auto Jump (LocalPlayer.sendPosition), the slowdown of an
    /// item in use (LocalPlayer.modifyInput), the helmet's Respiration
    /// (decreaseAirSupply), the wings, leather boots on powder snow
    /// (PowderSnowBlock.canEntityWalkOnPowderSnow), freeze-immune wear
    /// (LivingEntity.canFreeze), the dimension's FAST_LAVA, the sleeping
    /// pose and blindness (Player.isMobilityRestricted, which stops a sprint).
    /// The window's or the guest's own body reads the item in its selected slot
    /// (`getMainHandItem`) for the shapes that depend on it.
    pub(crate) fn sync_main_hand(&mut self) {
        let held = self
            .inventory
            .get(self.selected)
            .and_then(Option::as_ref)
            .filter(|stack| stack.count > 0)
            .map(|stack| stack.item.clone());
        if let Some(player) = self.player.as_mut() {
            player.main_hand = held;
        }
    }
    pub(crate) fn sync_player_body(&mut self) {
        self.sync_main_hand();
        let glider = self.glider_worn();
        let sleeping = self.sleeping.is_some();
        let item_in_use = self
            .using
            .and_then(|(slot, _)| {
                if slot == shield::OFFHAND_SLOT {
                    self.offhand.as_ref()
                } else {
                    self.inventory.get(slot)?.as_ref()
                }
            })
            .and_then(|s| self.items().get(&s.item));
        let item_use_speed = item_in_use.map(|i| i.use_speed);
        let item_use_slows_sprint = item_in_use.is_some_and(|i| !i.can_sprint);
        let respiration = self.armor[0]
            .as_ref()
            .map_or(0, |s| s.enchantment_level("minecraft:respiration") as i32);
        let leather_boots = worn_boots::leather_boots(&self.armor);
        let freeze_immune = self.wears_freeze_immune();
        let nudge = self.closest_space_nudge();
        let fast_lava = self.dimension_type().fast_lava;
        let blind = self.potions.effects.get("blindness").is_some();
        let Some(player) = self.player.as_mut() else {
            return;
        };
        player.nudge = nudge;
        player.mobility_restricted = blind;
        player.yaw = self.yaw;
        player.pitch = self.pitch;
        player.auto_jump_enabled = self.settings.auto_jump;
        player.item_use_speed = item_use_speed;
        player.item_use_slows_sprint = item_use_slows_sprint;
        player.respiration = respiration;
        player.glider = glider;
        // On someone else's server the glide flag is the server's
        // (`DATA_SHARED_FLAGS_ID`, applied by the SetEntityData handler); the
        // game's own integrated server has no such round trip, so the client
        // keeps predicting the clear a tick after the landing glide.
        player.server_owns_glide = self.integrated.is_none();
        let _ = player.set_sleeping(sleeping);
        player.leather_boots = leather_boots;
        player.freeze_immune = freeze_immune;
        player.fluids.fast_lava = fast_lava;
    }
    /// Joins an item only after drop hooks have seen its constructor's motion.
    fn add_item_entity_prepared(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        velocity: Option<[f64; 3]>,
        pickup_delay: i32,
        random: Option<spill::ItemRandom>,
    ) -> [f64; 3] {
        let velocity = self.push_item_entity(stack, position, velocity, pickup_delay, random);
        let _ = self.join_item_entity(self.next_entity, position);
        velocity
    }
    /// The block entity at `pos` saved with its id and position
    /// (`saveWithFullMetadata`), or the saved one Sunrise keeps as loaded:
    /// what a broken block's loot sees as `LootContextParams.BLOCK_ENTITY`.
    fn loot_block_entity(&self, pos: [i32; 3], block: &str) -> Option<sunrise_core::nbt::Tag> {
        let key = (pos[0], pos[1], pos[2]);
        match self.world.block_entities.get(&key) {
            Some(entity) => {
                let items = self.res.items();
                let current = self.encode_live_block_entity(key, entity, block, items);
                Some(spill::live_full_metadata(
                    &self.world,
                    key,
                    entity,
                    block,
                    items,
                    current,
                ))
            }
            None => spill::saved_entity(&self.world, pos),
        }
    }
    /// Where a loot roll at `origin` happens, for a mod's own loot kinds:
    /// the block entity it may copy from, and this level as
    /// `LootContext.getLevel` gives it (`Level.isRaining`: rain above 0.2;
    /// `isThundering`: thunder above 0.9; neither where `canHaveWeather` is
    /// false: a level with a ceiling or no sky light, or the End).
    fn loot_place_at(
        &self,
        origin: [f64; 3],
        block_entity: Option<sunrise_core::nbt::Tag>,
    ) -> sunrise_world::loot::LootPlace {
        let (block_entity, block_components) = match block_entity {
            Some(saved) => {
                let source = spill::source_snapshot(saved, None, None);
                (Some(source.nbt), Some(source.components))
            }
            None => (None, None),
        };
        let (rain, thunder) = self.world.weather.levels(1.0);
        // Level.canHaveWeather: sky light, no ceiling and not the End.
        let weather =
            self.world.shape.sky_light && !self.world.shape.has_ceiling && self.dim != Dim::End;
        sunrise_world::loot::LootPlace {
            origin: Some(origin),
            block_entity,
            block_components,
            storage: Some(self.score.storage.snapshot()),
            level: Some(sunrise_world::loot::LootLevel {
                dimension: self.dim.id().to_owned(),
                game_time: self.world.game_time,
                day_time: self.world.day_time,
                raining: weather && rain > 0.2,
                thundering: weather && thunder > 0.9,
                clocks: self.loot_clock_ticks(),
            }),
        }
    }
    /// `Block.getDrops` for a block a player breaks: the block's loot table
    /// with the player as `this`, drawing from its server random sequence
    /// (`LootContext.Builder.create`), else the level random. The roll's
    /// origin is the block's centre and `pending` retains its block entity.
    fn block_loot_at(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        held: Option<&ItemStack>,
    ) -> Vec<ItemStack> {
        let dynamic = self.pending.dynamic(pos);
        let place = self.loot_place_at(pos.map(|v| f64::from(v) + 0.5), None);
        let (_, entity) = self.native_clicker();
        let mut drops = sunrise_world::loot::with_native_loot_place(place, || {
            let roll = sunrise_client::items::BlockLootRoll {
                held,
                dynamic: &dynamic,
                breaker: true,
                ..Default::default()
            };
            self.roll_block_loot(pos, state, Some(entity as u32), roll)
        });
        let player = self.native_acting_id();
        self.native_block_loot(pos, state, Some(player), held, None, &mut drops);
        drops
    }
    /// `Player.drop`: the statistics and a thrown item entity in front of the player.
    pub(super) fn player_drop(&mut self, thrown: ItemStack) -> Option<u32> {
        let eye = self.eye_position()?;
        // LivingEntity.drop: every drop swings the main hand, a menu's
        // THROW and a creative tab's drop included.
        self.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
        // ServerPlayer.drop: the drop statistics, only when an item entity came of it.
        let (item, count) = (thrown.item.clone(), thrown.count as i32);
        let dropped = self.throw_from_eyes(thrown, eye, false, true)?;
        self.award_stat("dropped", &item, count);
        self.award_custom_stat("drop", 1);
        Some(dropped)
    }
    /// `Player.drop(stack, false, thrown_from_hand)`: the stack thrown from the eyes along the
    /// view, the main hand swinging for every drop (`LivingEntity.drop`; `PREDICTED` only keeps
    /// the swing from being sent back to the player who made it). `thrown_from_hand` names the
    /// player as the thrower, as the drop key does. The item's id, none for an empty stack or a
    /// toss a mod canceled.
    pub(super) fn drop_stack_from_eyes(
        &mut self,
        stack: ItemStack,
        thrown_from_hand: bool,
    ) -> Option<u32> {
        if stack.count == 0 {
            return None;
        }
        self.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
        let eye = self.eye_position()?;
        self.throw_from_eyes(stack, eye, false, thrown_from_hand)
    }
    /// The UUID of the player acting: the guest whose turn it is, else the host.
    pub(super) fn acting_player_uuid(&mut self) -> [i32; 4] {
        match self.lan_acting_uuid {
            Some(guest) => sunrise_client::lan::playerdata::uuid_ints(guest),
            None => self.level.ensure_player_uuid(),
        }
    }
    /// `Entity.doWaterSplashEffect` for a dropped item. The client draws the
    /// bubble and splash particles (`Level.addParticle` is empty on a server).
    /// The server plays `entity.generic.splash` (`Entity.playSound`); a
    /// client's `playSeededSound` ignores that call.
    pub(super) fn apply_item_splash(&mut self, splash: sunrise_client::entities::ItemSplash) {
        use sunrise_client::entities::ItemSplash;
        let client = self.options.execution_role.is_client();
        match splash {
            ItemSplash::Sound { at, volume, pitch } if !client => {
                self.play_audio("minecraft:entity.generic.splash", Some(at), volume, pitch);
            }
            ItemSplash::Particle {
                bubble,
                at,
                velocity,
            } if client => {
                use sunrise_client::particles::{ParticleOptions, ParticleType};
                let kind = if bubble {
                    ParticleType::Bubble
                } else {
                    ParticleType::Splash
                };
                self.particles
                    .add(&ParticleOptions::simple(kind), at, velocity);
            }
            _ => {}
        }
    }
    /// `Block.fallOn` for item `id`, which fell `distance` onto the block at `on`
    /// (`Entity.checkFallDamage`): a mod block's `fall_on` Part hears it, through the same
    /// `block_contact::fall` as a player's and a mob's landing. An item takes no fall damage.
    /// Nothing is lent or built unless a Part of that block listens.
    fn item_fall_on(&mut self, id: u32, on: sunrise_core::position::BlockPos, distance: f64) {
        let state = self.world.block(on.x, on.y, on.z);
        if !sunrise_client::block_contact::fall_listens(state) {
            return;
        }
        let mut lent = sunrise_client::health::LentLevel::take(&mut self.world);
        // A native fall_on Part can unwind. Keep the persistent level carrier
        // outside the callback's temporary views, and return its progressed state
        // before propagating the original panic. This does not restore the item batch.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut level = sunrise_client::health::DamageLevel::lent(
                &self.world,
                self.pack.assets.registry,
                &mut lent,
                Some(world_items!(self)),
                Some(&self.pack.block_registries),
                &self.damage_hooks,
            )
            .with_effects(crate::game::entity::hit_effects_of!(self, None));
            let contact = sunrise_client::block_contact::Contact {
                entity: id as i32,
                kind: "minecraft:item",
                state,
                position: on,
            };
            let _ = sunrise_client::block_contact::fall(&mut level, contact, distance, 0.0);
        }));
        lent.give_back(&mut self.world);
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    }
    /// The item entities `tick_dropped_items` took out, back in the level,
    /// before those its hooks spawned meanwhile (`addFreshEntity` appends).
    /// They keep their join numbers; the spawned ones got theirs on joining.
    fn put_back_dropped(&mut self, items: sunrise_client::join_seq::JoinList<ItemEntity>) {
        self.dropped.restore(
            items,
            sunrise_client::join_seq::RestoreOrder::SurvivorsFirst,
        );
    }
    /// Run vanilla block behaviour against the world through `LevelAccess`.
    fn with_level<R>(
        &mut self,
        f: impl FnOnce(&Behaviours, &mut GameLevel<'_>, &mut Ticks) -> R,
    ) -> R {
        self.ensure_stateful_entity_table();
        let result = self.with_level_access(false, f);
        self.flush_spill();
        if self.lan.is_some() {
            self.flush_lan_particles();
            self.flush_native_sounds();
        }
        result
    }

    /// Placement queries need neither trigger snapshots nor live factory setup or effect drains.
    fn with_placement_preview_level<R>(
        &mut self,
        f: impl FnOnce(&Behaviours, &GameLevel<'_>, &Ticks) -> R,
    ) -> R {
        self.with_level_access(true, |behaviours, level, ticks| f(behaviours, level, ticks))
    }

    fn with_level_access<R>(
        &mut self,
        placement_preview: bool,
        f: impl FnOnce(&Behaviours, &mut GameLevel<'_>, &mut Ticks) -> R,
    ) -> R {
        let darken = self.sky_darken();
        let (rain, thunder) = self.weather_levels(1.0);
        let (
            player_box,
            player_collision,
            cart_boxes,
            arrow_boxes,
            guest_boxes,
            building_player_boxes,
            fire_players,
            trigger_hooks,
            trigger_extras,
            open_containers,
            golem_chests,
        ) = if placement_preview {
            Default::default()
        } else {
            // Spectators ignore block triggers (Entity.isIgnoringBlockTriggers).
            let spectator = self.spectator();
            let player_box = self.player.as_ref().filter(|_| !spectator).map(|p| {
                let w = f64::from(p.dimensions.width) / 2.0;
                (
                    [p.position.x - w, p.position.y, p.position.z - w],
                    [
                        p.position.x + w,
                        p.position.y + f64::from(p.dimensions.height),
                        p.position.z + w,
                    ],
                )
            });
            let player_collision = self.player.as_ref().filter(|_| !spectator).map(|p| {
                let main_hand = self.inventory.get(self.selected).and_then(Option::as_ref);
                PlayerCollision {
                    y: p.position.y,
                    descending: p.velocity.y < 0.0,
                    fall_distance: p.fall_distance,
                    main_hand_item: main_hand.map_or_else(
                        || Arc::from("minecraft:air"),
                        |stack| Arc::clone(&stack.item),
                    ),
                    main_hand_count: main_hand.map_or(0, |stack| stack.count as i32),
                    leather_boots: worn_boots::leather_boots(&self.armor),
                }
            });
            let cart_boxes = self
                .carts
                .iter()
                .map(|c| {
                    let p = c.position;
                    (
                        [p.x - 0.49, p.y, p.z - 0.49],
                        [p.x + 0.49, p.y + 0.7, p.z + 0.49],
                    )
                })
                .collect();
            let arrow_boxes = self
                .creatures
                .projectiles
                .iter()
                .filter(|p| p.arrow.is_some())
                .map(|p| p.position)
                .chain(self.tridents.iter().map(|t| t.position))
                .map(|at| {
                    let b = sunrise_client::arrows::arrow_bounds(at);
                    (b.min_position().components(), b.max_position().components())
                })
                .collect();
            let guest_boxes = self.lan_block_trigger_boxes();
            let building_player_boxes = self
                .lan_acting
                .is_some()
                .then(|| self.lan_building_boxes.boxes(self.dim));
            let fire_players: Vec<[f64; 3]> = self
                .player
                .iter()
                .filter(|_| !spectator)
                .map(|p| [p.position.x, p.position.y, p.position.z])
                .chain(self.lan.iter().flat_map(|h| {
                    h.hub
                        .guests
                        .simulation_iter()
                        .filter(|g| h.hub.here(g) && !g.spectator())
                        .map(|g| g.position)
                }))
                .collect();
            let mut trigger_hooks: Vec<(Aabb, bool)> = self
                .hook
                .iter()
                .map(|h| (h.bounds(), h.applies_block_effects()))
                .collect();
            self.lan_hook_triggers(&mut trigger_hooks);
            // EntityType.LIGHTNING_BOLT: 0 x 0.
            let trigger_extras: Vec<Aabb> = self
                .bolts
                .iter()
                .map(|b| {
                    let at = Vec3::new(b.at[0], b.at[1], b.at[2]);
                    Aabb::new(at, at)
                })
                .collect();
            // `ContainerOpenersCounter.getEntitiesWithContainerOpen`: the players
            // whose menu still holds a container of the block at a position. A
            // double chest answers for both halves (`ChestBlockEntity$1` takes a
            // `CompoundContainer`), and a spectator's menu counts for nothing.
            let mut open_containers: Vec<(i32, i32, i32)> = Vec::new();
            for key in self.container.into_iter().filter(|_| !spectator).chain(
                self.lan
                    .iter()
                    .flat_map(|h| {
                        h.hub
                            .guests
                            .simulation_iter()
                            .filter(|g| h.hub.here(g) && !g.spectator())
                    })
                    .filter_map(|g| g.menu.as_ref().and_then(|m| m.pos)),
            ) {
                open_containers.push(key);
                if let Some(other) = self.chest_other_half(key) {
                    open_containers.push(other);
                }
            }
            // CopperGolem.hasContainerOpen: the chest a golem opened, and its other half.
            let mut golem_chests: Vec<(i32, i32, i32)> = Vec::new();
            for &key in self.world.chest_openers.keys() {
                golem_chests.push(key);
                golem_chests.extend(self.chest_other_half(key));
            }
            (
                player_box,
                player_collision,
                cart_boxes,
                arrow_boxes,
                guest_boxes,
                building_player_boxes,
                fire_players,
                trigger_hooks,
                trigger_extras,
                open_containers,
                golem_chests,
            )
        };
        let dimension = self.dim.level();
        let rules = self.level_rules();
        let native_authoritative = self.native_authoritative();
        let liquid_transaction_open = self.has_level_transaction();
        let environment = Self::level_environment(
            self.remote.as_ref(),
            &self.res,
            self.dim.id(),
            &self.dimension_type().id,
        );
        let other_entities = entity::entity_lists_of!(self, without_carts);
        let loot_clocks = self.loot_clock_ticks();
        let mut level = GameLevel {
            cake_probe: false,
            cake_requested: std::cell::Cell::new(false),
            cake_answer: None,
            api_level: if native_authoritative {
                self.level(self.dim.id())
            } else {
                self.native_client_level()
            },
            part_state_index: self.attachment_indexes.get(&self.dim),
            attachment_query_scopes: &self.transactions.attachment_query_scopes,
            command_block_transition: None,
            positions: piston_carry_state::PositionState {
                gateways: &mut self.be_anims.gateway_servers,
                hoppers: &mut self.hoppers,
            },
            border: self.border.geometry(),
            redstone_experiments: self
                .level
                .enabled_features
                .iter()
                .any(|feature| feature == "minecraft:redstone_experiments"),
            cart_boxes,
            item_frames: &self.item_frames,
            carts: &mut self.carts,
            arrow_boxes,
            other_entities,
            crystals: &self.crystals,
            trigger_hooks,
            trigger_extras,
            world: &mut self.world,
            facts: self.res.facts(),
            tables: native_block_tables::BlockTables::new(&self.res, &self.pack),
            collision: self.collision,
            events: &mut self.level_events,
            dimension,
            darken,
            environment,
            raining: rain > 0.2,
            thundering: thunder > 0.9,
            enabled_features: self
                .remote
                .as_ref()
                .map_or(self.level.enabled_features.as_arc(), |remote| {
                    &remote.features
                }),
            player_box,
            guest_boxes,
            building_player_boxes,
            player_collision,
            biomes: world_biomes!(self),
            biome_keys: (self.serving.is_some() || self.options.dedicated.is_some())
                .then(|| self.world_tables.as_ref().map(|t| &*t.data_maps.biome_keys))
                .flatten(),
            biome_zoom: self.streaming.biome_zoom,
            seed: self.level.seed,
            behaviours: self.res.behaviours(),
            items: world_items!(self),
            recipes: world_recipes!(self),
            // A client rolls no loot (LootTable.getRandomItems needs a
            // ServerLevel): it doesn't load the tables on its game thread.
            loot: if self.remote.is_some() {
                None
            } else {
                self.server_loot.get()
            },
            loot_storage: self.score.storage.snapshot(),
            loot_clocks,
            registry: self.pack.assets.registry,
            block_registries: &self.pack.block_registries,
            rules,
            difficulty: i32::from(self.ui.difficulty),
            fire_players,
            rand_sequences: &mut self.rand_sequences,
            pending: &mut self.pending,
            spill: &mut self.spill,
            tickets: self.tickets.levels(),
            random_tick_sections: &mut self.random_tick_sections,
            lid_openers: &mut self.lid_openers,
            open_containers: &open_containers,
            golem_chests: &golem_chests,
            client_side: self.remote.is_some(),
            native_authoritative,
            native_player: self.lan_acting_uuid,
            native_player_entity: self.lan_acting_entity.map(|id| id as i32),
            liquid_transaction_open,
            debug: self.world_debug,
            native_sounds: &mut self.creatures.lan_sounds,
            loop_context: &self.loop_context,
            side: game_side!(self),
            native_particles: &mut self.creatures.lan_particles,
            sound_seeds: self.lan.as_ref().map(|host| &host.hub),
            drop_tool: None,
            drop_entity: None,
            interacting: None,
            interacting_tool: None,
            location: location_world::LocationRefs {
                dimension: self.dim.id(),
                end: &self.end,
                tags: advancements::TagSources {
                    pack_tags: self.world_tables.as_ref().map(|t| &t.pack_tags),
                    items: world_items!(self),
                    mob_data: self.res.mob_data(),
                    damage: &self.health_resources.damage,
                    resources: &self.options.resources,
                },
            },
            drop_breaker_at: None,
            drop_into: None,
        };
        f(self.res.behaviours(), &mut level, &mut self.block_ticks)
    }
    pub(super) fn native_block_ignite_allowed(
        &mut self,
        pos: BlockPos,
        cause: &'static str,
    ) -> bool {
        self.with_level(|_, level, _| level.allow_block_ignite(pos, cause))
    }
    pub(super) fn native_cauldron_level_change_allowed(
        &mut self,
        pos: BlockPos,
        from: i32,
        to: i32,
        reason: &'static str,
    ) -> bool {
        self.with_level(|_, level, _| level.allow_cauldron_level_change(pos, from, to, reason))
    }
    /// Level.setBlock(pos, state, UPDATE_ALL) with neighbour/shape updates.
    fn level_set(&mut self, pos: [i32; 3], state: BlockStateId) -> Result<()> {
        self.level_set_with_flags(pos, state, level_flags::UPDATE_ALL)
    }
    /// Level.setBlock with the caller's update flags.
    fn level_set_with_flags(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> Result<()> {
        self.set_block_live(pos, state, flags, false);
        self.process_level_events()
    }

    /// `Level.setBlock` with the full server available at the block-entity drop site.
    fn set_block_live(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        flags: i32,
        create_entity: bool,
    ) -> bool {
        self.set_block_live_with_entity(
            pos,
            state,
            flags,
            create_entity,
            transactions::BlockEntityCreation::Live,
        )
    }

    fn set_block_live_with_entity(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        flags: i32,
        create_entity: bool,
        creation: transactions::BlockEntityCreation,
    ) -> bool {
        use transactions::BlockEntityCreation::{Deferred, Live, Staged};
        let live_factory = matches!(creation, Live);
        let [x, y, z] = pos;
        let at = BlockPos::new(x, y, z);
        let previous = self.world.block(x, y, z);
        let b = self.res.behaviours();
        let server_removal = self.remote.is_none()
            && flags & level_flags::UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS == 0
            && b.block(previous) != b.block(state)
            && !b.keeps_block_entity(previous, state)
            && (self.res.facts().get(previous).block_entity
                || self.world.block_entities.stateful_entity(previous.0)
                || self.world.block_entities.get(&(x, y, z)).is_some());
        if !server_removal && live_factory {
            return self.with_level(|b, level, ticks| {
                let changed = b.set_block(level, ticks, at, state, flags);
                if create_entity {
                    level.finish_native_block(pos, previous, state, changed);
                }
                changed
            });
        }
        self.ensure_stateful_entity_table();
        let Some((write, removal, spawns, orbs)) =
            self.with_level_access(false, |b, level, ticks| {
                let write = b.begin_block_write(level, ticks, at, state, flags)?;
                let spawn_start = level.spill.spawns.len();
                let orb_start = level.spill.orbs.len();
                let removal = if server_removal {
                    write
                        .effectful_removal()
                        .map(|old| level.pre_remove_entity(at, old, true))
                } else {
                    None
                };
                let spawns = level.spill.spawns.split_off(spawn_start);
                let orbs = level.spill.orbs.split_off(orb_start);
                Some((write, removal, spawns, orbs))
            })
        else {
            return false;
        };
        self.publish_removal_spawns(spawns, orbs);
        self.with_level(|b, level, ticks| {
            let removal_updates =
                removal.map(|removal| level.finish_remove_entity(at, previous, removal));
            let changed =
                b.finish_block_write(level, ticks, write, removal_updates, |level, at, state| {
                    match creation {
                        Live => level.ensure_block_entity(at, state),
                        Deferred => {}
                        Staged { entity, .. } => {
                            entity.install(level.world, (at.x, at.y, at.z), true)
                        }
                    }
                });
            if create_entity && live_factory {
                level.finish_native_block(pos, previous, state, changed);
            }
            changed
        })
    }
    fn attack_block(&mut self, pos: [i32; 3]) -> Result<()> {
        let note = self.with_level(|b, level, ticks| {
            b.attack_block(level, ticks, BlockPos::new(pos[0], pos[1], pos[2]))
        });
        // NoteBlock.attack awards the statistic.
        if note {
            self.award_custom_stat("play_noteblock", 1);
        }
        self.process_level_events()
    }
    /// `AbstractCandleBlock.getParticleOffsets`, in sixteenths of a block:
    /// `CandleBlock.PARTICLE_OFFSETS` by its `candles` (one to four wicks),
    /// then `CandleCakeBlock.PARTICLE_OFFSETS`, whose one wick sits a block up.
    fn candle_offsets(name: &str, candles: Option<usize>) -> &'static [[u8; 3]] {
        const OFFSETS: [&[[u8; 3]]; 5] = [
            &[[8, 8, 8]],
            &[[6, 7, 8], [10, 8, 7]],
            &[[8, 5, 10], [6, 7, 8], [9, 8, 7]],
            &[[7, 5, 9], [10, 7, 9], [6, 7, 6], [9, 8, 6]],
            &[[8, 16, 8]],
        ];
        if name.ends_with("candle_cake") {
            OFFSETS[4]
        } else {
            OFFSETS[candles.unwrap_or(1).clamp(1, 4) - 1]
        }
    }
    /// `AbstractCandleBlock.extinguish`: the smoke from each wick, at the
    /// block's `getParticleOffsets` (sixteenths of a block), rising 0.1.
    fn candle_smoke(&mut self, pos: [i32; 3], state: BlockStateId) {
        use sunrise_client::particles::{ParticleOptions, ParticleType};
        let candles = self
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("candles")?.parse::<usize>().ok());
        let offsets = Self::candle_offsets(&self.res.facts().get(state).name, candles);
        for o in offsets {
            self.particles.add(
                &ParticleOptions::simple(ParticleType::Smoke),
                [
                    f64::from(pos[0]) + f64::from(o[0]) / 16.0,
                    f64::from(pos[1]) + f64::from(o[1]) / 16.0,
                    f64::from(pos[2]) + f64::from(o[2]) / 16.0,
                ],
                // AbstractCandleBlock.extinguish: ySpeed 0.1F, widened.
                [0.0, f64::from(0.1_f32), 0.0],
            );
        }
    }
    /// `LeverBlock.makeParticle` at full scale: a powered lever puffs one red
    /// dust on the clicking client (`LeverBlock.useWithoutItem`).
    fn lever_puff(&mut self, pos: [i32; 3]) {
        // The server's side of a guest's pull draws nothing.
        if self.lan_acting.is_some() {
            return;
        }
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let Some(registry_state) = self.pack.assets.registry.state(state) else {
            return;
        };
        let prop = |key: &str| registry_state.properties.get(key).map(String::as_str);
        if prop("powered") != Some("true") {
            return;
        }
        // FACING is horizontal; getConnectedDirection is the face's direction.
        let facing = match prop("facing") {
            Some("north") => [0, 0, -1],
            Some("south") => [0, 0, 1],
            Some("west") => [-1, 0, 0],
            Some("east") => [1, 0, 0],
            _ => return,
        };
        let connected = match prop("face") {
            Some("ceiling") => [0, -1, 0],
            Some("floor") => [0, 1, 0],
            _ => facing,
        };
        let at = |base: i32, i: usize| {
            f64::from(base) + 0.5 + 0.1 * f64::from(-facing[i]) + 0.2 * f64::from(-connected[i])
        };
        let p = [at(pos[0], 0), at(pos[1], 1), at(pos[2], 2)];
        self.particles.add(
            &sunrise_client::particles::ParticleOptions::dust([1.0, 0.0, 0.0], 1.0),
            p,
            [0.0; 3],
        );
    }
    /// `DistanceManager`'s tickets of the level in place, from every player in
    /// it (the local player, LAN guests, a server's players; a spectator the
    /// rule skips holds none), its forced chunks and its portal tickets. A game
    /// with neither a server nor a player (the headless fixtures) ticks
    /// everywhere.
    fn update_chunk_tickets(&mut self) {
        self.rebuild_chunk_tickets(true);
    }

    fn rebuild_chunk_tickets(&mut self, notify: bool) {
        let rule = self
            .ui
            .rules
            .bool_at(sunrise_client::gamerules::vanilla::SPECTATORS_GENERATE_CHUNKS);
        let mut players: Vec<((i32, i32), bool)> = self.lan.as_ref().map_or_else(Vec::new, |h| {
            h.hub
                .guests
                .iter()
                .filter(|g| h.hub.here(g))
                .filter(|g| !g.is_fake()) // fake players hold small forced tickets below
                .map(|g| (g.ticket_chunk(), g.spectator()))
                .collect()
        });
        if let Some(p) = &self.player {
            players.push((
                (
                    (p.position.x.floor() as i32) >> 4,
                    (p.position.z.floor() as i32) >> 4,
                ),
                self.spectator(),
            ));
        }
        // A spectator the rule skips is still a player: a game that has one
        // must not take the headless fixtures' tick-everywhere case below.
        let any_player = !players.is_empty();
        let sources = (!self.options.execution_role.is_client() || any_player).then(|| {
            sunrise_client::ticket_levels::Sources {
                players: players
                    .into_iter()
                    .filter(|&(_, spectator)| game_mode::loads_chunks(rule, spectator))
                    .map(|(chunk, _)| chunk)
                    .collect(),
                forced: self.streaming.forced.iter().copied().collect(),
                // Filled from the `PORTAL` tickets by `update`.
                portal: Vec::new(),
                spawn: Vec::new(),
                // EnderDragonFight.tick: while a player watches the fight, the arena holds a
                // `DRAGON` ticket of radius 9 at (0, 0), so its island ticks out to 7 chunks
                // whatever the watcher's simulation distance. The fight is where the type has one.
                dragon: (self.dimension_type().has_ender_dragon_fight
                    && self.fight.arena_watched())
                .then_some((0, 0)),
                viewed: {
                    let mut viewed = self.viewed_tickets();
                    if self.streaming.has_async_requests() {
                        viewed.extend(self.streaming.async_requests().map(|center| {
                            sunrise_client::ticket_levels::ViewedTicket {
                                center,
                                radius: 0,
                                ticking: false,
                            }
                        }));
                        viewed.sort_unstable();
                        viewed.dedup();
                    }
                    viewed
                },
                view_distance: self.settings.render_distance.clamp(2, 32),
                simulation_distance: self.settings.simulation_distance.max(2),
            }
        });
        // The levels' changes are kept only for a listener, so that a tick
        // with none looks at no chunk.
        self.tickets.record_changes(
            notify
                && self.is_logical_server()
                && mod_api::__private::listened(mod_api::HookId::ChunkTicketLevelUpdated),
        );
        self.tickets.update(sources);
        if notify {
            self.run_chunk_ticket_level_updates();
        }
    }
    /// `ServerLevel.startTickingChunk`: a loaded chunk's saved ticks are
    /// scheduled from the game time at which it becomes a ticking chunk
    /// (block ticking, with its neighbours loaded). Chunks become ticking
    /// between ticks in vanilla: before this tick's clock advanced.
    fn start_ticking_chunks(&mut self) {
        let levels = self.tickets.levels();
        let world = &self.world;
        let started = self
            .tick_chunks
            .start_ticking(|(x, z)| levels.loaded_ticking(x, z) && ticking_chunk(world, x, z));
        let time = self.world.game_time - 1;
        for (x, z) in started {
            let pos = sunrise_core::position::ChunkPos::new(x, z);
            self.block_ticks.blocks.unpack_chunk(pos, time);
            self.block_ticks.fluids.unpack_chunk(pos, time);
        }
    }

    /// Copies the moving-piston block entities into the world, where the
    /// collision queries and the renderer read them.
    ///
    /// A client runs this for every block change that ends a piston's move,
    /// so it stays linear in the pistons moving: a clock of a thousand pistons
    /// ends two thousand moves at once.
    fn sync_moving_pistons(&mut self) {
        use sunrise_client::fast_hash::{FxHashMap, FxHashSet};
        let previous = std::mem::take(&mut self.world.moving_pistons);
        let current: Vec<_> = self
            .block_ticks
            .moving_pistons
            .iter()
            .map(|(p, piston)| ([p.x, p.y, p.z], piston.shape_state()))
            .collect();
        // The positions drawn by `current` or `fallback` so far.
        let mut drawn: FxHashSet<[i32; 3]> = current.iter().map(|(p, _)| *p).collect();
        let mut fallback: Vec<_> = previous
            .into_iter()
            .filter(|(pos, _)| !drawn.contains(pos))
            .collect();
        drawn.extend(fallback.iter().map(|(p, _)| *p));
        // A finished piston stays drawn until the chunk mesh shows the block
        // it placed, however many ticks that takes. Pistons share a few
        // sections, so each section is asked once.
        let older = std::mem::take(&mut self.world.moving_piston_fallback);
        let mut pending: FxHashMap<(i32, i32, i32), bool> = FxHashMap::default();
        for (pos, piston) in older {
            if drawn.contains(&pos) {
                continue;
            }
            let Some(section) = self.section_of(pos) else {
                continue;
            };
            if *pending
                .entry(section)
                .or_insert_with(|| self.section_mesh_pending_in(section))
            {
                drawn.insert(pos);
                fallback.push((pos, piston));
            }
        }
        self.world.moving_piston_fallback = fallback;
        self.world.moving_pistons = current;
    }
    /// Side effects reported by the behaviour library, in vanilla order.
    fn process_level_events(&mut self) -> Result<()> {
        let events = std::mem::take(&mut self.level_events);
        self.lan_record_events(&events);
        dispense_fixture::record_events(&events);
        for event in events {
            match event {
                LevelEvent::LevelEvent {
                    id: 2001,
                    pos,
                    data,
                    ..
                } => {
                    // LevelEventHandler plays the block's break sound before asking for debris.
                    let state = BlockStateId(data as u32);
                    let p = [pos.x, pos.y, pos.z];
                    self.play_break_sound(p, state);
                    self.destroy_particles(p, state);
                }
                LevelEvent::LevelEvent {
                    id: 3006,
                    pos,
                    data,
                    ..
                } => self.sculk_charge_event(pos, data),
                LevelEvent::LevelEvent { id: 3007, pos, .. } => self.shriek_event(pos),
                // LevelEventHandler 1010 and 1011: a jukebox's song starts
                // (`playJukeboxSong`) or stops, and the parrots nearby dance or stop.
                LevelEvent::LevelEvent {
                    id: 1010,
                    pos,
                    data,
                    ..
                } => self.jukebox_song_event([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent { id: 1011, pos, .. } => {
                    self.jukebox_stop_event([pos.x, pos.y, pos.z])
                }
                // LevelEventHandler 2019 and 2020: the crack on a face of the
                // block being mined, with its hit sound for 2020.
                LevelEvent::LevelEvent {
                    id: id @ (2019 | 2020),
                    pos,
                    data,
                    ..
                } => {
                    let face = sunrise_core::geometry::Direction::ALL[data.clamp(0, 5) as usize];
                    self.breaking_block_effects([pos.x, pos.y, pos.z], face.normal(), id == 2020);
                }
                LevelEvent::LevelEvent {
                    id: 3008,
                    pos,
                    data,
                    ..
                } => self.brush_complete_event([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent { id: 2004, pos, .. } => {
                    self.spawner_burst([pos.x, pos.y, pos.z])
                }
                // LevelEventHandler 1505: the growth particles, then the sound.
                LevelEvent::LevelEvent {
                    id: 1505,
                    pos,
                    data,
                    ..
                } => {
                    self.growth_particles(pos, data);
                    let at = [pos.x, pos.y, pos.z].map(|v| f64::from(v) + 0.5);
                    self.play_audio("minecraft:item.bone_meal.use", Some(at), 1.0, 1.0);
                }
                LevelEvent::LevelEvent {
                    id: 2015,
                    pos,
                    data,
                    ..
                } => self.dragon_egg_particles([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent {
                    id: 2016,
                    pos,
                    data,
                    ..
                } => self.shulker_teleport_particles([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent {
                    id: 2017,
                    pos,
                    data,
                    ..
                } => self.consume_teleport_particles([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent {
                    id: 2018,
                    pos,
                    data,
                    ..
                } => self.enderman_teleport_particles([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent { id: 2003, pos, .. } => {
                    self.eye_break_particles([pos.x, pos.y, pos.z]);
                }
                LevelEvent::LevelEvent {
                    id: 2013,
                    pos,
                    data,
                    ..
                } => self.smash_dust(pos, data),
                // LevelEventHandler 2002 and 2007: a potion's splash.
                LevelEvent::LevelEvent {
                    id: 2002,
                    pos,
                    data,
                    ..
                } => self.potion_splash_particles(pos, data, false),
                LevelEvent::LevelEvent {
                    id: 2007,
                    pos,
                    data,
                    ..
                } => self.potion_splash_particles(pos, data, true),
                LevelEvent::LevelEvent {
                    id: id @ 3002..=3005,
                    pos,
                    data,
                    ..
                } => self.block_particle_event(id, pos, data),
                LevelEvent::LevelEvent {
                    id: 1500,
                    pos,
                    data,
                    ..
                } => self.composter_fill_event(pos, data == 1),
                // LevelEventHandler 1504: the stalactite's drip particle.
                LevelEvent::LevelEvent { id: 1504, pos, .. } => self.dripstone_drip_particle(pos),
                // LevelEventHandler 2006: the dragon fireball's burst.
                LevelEvent::LevelEvent {
                    id: 2006,
                    pos,
                    data,
                    ..
                } => self.dragon_fireball_burst(pos, data),
                // LevelEventHandler 2009: a wet sponge drying.
                LevelEvent::LevelEvent { id: 2009, pos, .. } => {
                    self.puffs_above_block(pos, sunrise_client::particles::ParticleType::Cloud)
                }
                // LevelEventHandler 2011 and 2012: a bee growing a crop, a
                // turtle egg laid.
                LevelEvent::LevelEvent {
                    id: 2011 | 2012,
                    pos,
                    data,
                    ..
                } => self.turtle_egg_particles(pos, data),
                // LevelEventHandler 2014: a straw bed destroyed, with no sound.
                LevelEvent::LevelEvent {
                    id: 2014,
                    pos,
                    data,
                    ..
                } => self.destroy_particles([pos.x, pos.y, pos.z], BlockStateId(data as u32)),
                // EnderDragonFight.spawnNewGateway: an always-visible burst
                // and a sound at the gateway for every nearby player.
                LevelEvent::LevelEvent { id: 3000, pos, .. } => {
                    let at = [
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ];
                    self.particles.add_with(
                        None,
                        &sunrise_client::particles::ParticleOptions::simple(
                            sunrise_client::particles::ParticleType::ExplosionEmitter,
                        ),
                        at,
                        [0.0; 3],
                        true,
                        true,
                    );
                    let random = self.particles.level_random();
                    let pitch = (1.0 + (random.next_float() - random.next_float()) * 0.2) * 0.7;
                    self.play_audio_unshared(
                        "minecraft:block.end_gateway.spawn",
                        Some(at),
                        10.0,
                        pitch,
                    );
                }
                // LevelEventHandler 3009: a sniffer egg cracking.
                LevelEvent::LevelEvent { id: 3009, pos, .. } => {
                    self.sniffer_egg_crack_particles(pos)
                }
                LevelEvent::LevelEvent {
                    id: id @ 3011..=3021,
                    pos,
                    data,
                    ..
                } => self.trial_level_event(id, [pos.x, pos.y, pos.z], data),
                // LevelEvent.SOUND_PORTAL_TRAVEL: LevelEventHandler.levelEvent
                // plays SimpleSoundInstance.forLocalAmbience (ambient, no
                // position). EndPortalBlock.entityInside travels through
                // TeleportTransition.PLAY_PORTAL_SOUND, which sends 1032 to
                // the traveling player. The packet's position is BlockPos.ZERO,
                // so a positional play would not be heard; at None is the
                // relative sound, and it is not captured for other players.
                LevelEvent::LevelEvent { id: 1032, .. } => {
                    let pitch = self.particles.level_random().next_float() * 0.4 + 0.8;
                    self.play_audio_on(
                        sunrise_client::sound_engine::AMBIENT,
                        "minecraft:block.portal.travel",
                        None,
                        0.25,
                        pitch,
                    );
                }
                LevelEvent::LevelEvent { id, pos, .. } => {
                    let c = Some([
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ]);
                    let sound = match id {
                        1503 => Some(("minecraft:block.end_portal_frame.fill", 1.0)),
                        1501 => Some(("minecraft:block.lava.extinguish", 0.5)),
                        1009 => Some(("minecraft:block.fire.extinguish", 0.5)),
                        1502 => Some(("minecraft:block.redstone_torch.burnout", 0.5)),
                        1033 => Some(("minecraft:block.chorus_flower.grow", 1.0)),
                        1034 => Some(("minecraft:block.chorus_flower.death", 1.0)),
                        1045 => Some(("minecraft:block.pointed_dripstone.land", 2.0)),
                        1052 => Some(("minecraft:block.sulfur_spike.land", 2.0)),
                        1046 => Some((
                            "minecraft:block.pointed_dripstone.drip_lava_into_cauldron",
                            2.0,
                        )),
                        1047 => Some((
                            "minecraft:block.pointed_dripstone.drip_water_into_cauldron",
                            2.0,
                        )),
                        // LevelEvent.SOUND_ZOMBIE_WOODEN_DOOR (attack, break).
                        1019 => Some(("minecraft:entity.zombie.attack_wooden_door", 2.0)),
                        1021 => Some(("minecraft:entity.zombie.break_wooden_door", 2.0)),
                        1026 => Some(("minecraft:entity.zombie.infect", 2.0)),
                        1027 => Some(("minecraft:entity.zombie_villager.converted", 2.0)),
                        1030 => Some(("minecraft:block.anvil.use", 1.0)),
                        1029 => Some(("minecraft:block.anvil.destroy", 1.0)),
                        1031 => Some(("minecraft:block.anvil.land", 0.3)),
                        1042 => Some(("minecraft:block.grindstone.use", 1.0)),
                        1044 => Some(("minecraft:block.smithing_table.use", 1.0)),
                        // LevelEvent.SOUND_PAGE_TURN (a lectern's page).
                        1043 => Some(("minecraft:item.book.page_turn", 1.0)),
                        // LevelEvent.SOUND_PHANTOM_BITE: a phantom's bite.
                        1039 => Some(("minecraft:entity.phantom.bite", 0.3)),
                        // LevelEvent.SOUND_SPLASH_POTION and its instant twin.
                        1053 | 1054 => Some(("minecraft:entity.splash_potion.break", 1.0)),
                        _ => None,
                    };
                    if let Some((name, volume)) = sound {
                        let pitch = match id {
                            1009 => {
                                2.6 + (self.world.random.next_float()
                                    - self.world.random.next_float())
                                    * 0.8
                            }
                            // LevelEventHandler draws these from the client
                            // level's random, before 1501's and 1502's smoke.
                            1501 | 1502 => {
                                let r = self.particles.level_random();
                                2.6 + (r.next_float() - r.next_float()) * 0.8
                            }
                            1039 | 1045 | 1046 | 1047 | 1052 | 1053 | 1054 => {
                                // LevelEventHandler draws these from the client
                                // level's random, not the server's.
                                self.particles.level_random().next_float() * 0.1 + 0.9
                            }
                            1029 | 1030 | 1042 | 1043 | 1044 => {
                                self.world.random.next_float() * 0.1 + 0.9
                            }
                            1026 | 1027 => {
                                let r = self.particles.level_random();
                                (r.next_float() - r.next_float()) * 0.2 + 1.0
                            }
                            1019 | 1021 => {
                                (self.world.random.next_float() - self.world.random.next_float())
                                    * 0.2
                                    + 1.0
                            }
                            _ => 1.0,
                        };
                        if matches!(
                            id,
                            1019 | 1021
                                | 1026
                                | 1027
                                | 1029
                                | 1030
                                | 1039
                                | 1045
                                | 1052
                                | 1053
                                | 1054
                                | 1501
                                | 1503
                        ) {
                            // Clients play these sounds from their level events;
                            // capturing it also sends a duplicate sound packet.
                            self.play_audio_unshared(name, c, volume, pitch);
                        } else {
                            self.play_audio(name, c, volume, pitch);
                        }
                    }
                    // LevelEventHandler draws smoke after the sound.
                    match id {
                        1501 => self.puffs_above_block(
                            pos,
                            sunrise_client::particles::ParticleType::LargeSmoke,
                        ),
                        1502 => self.torch_burnout_smoke(pos),
                        1503 => self.end_portal_frame_smoke(pos),
                        _ => {}
                    }
                }
                LevelEvent::ContainerLootTable { pos, tag } => {
                    let key = (pos.x, pos.y, pos.z);
                    if let Some(mut entity) = self.world.block_entities.remove(&key) {
                        self.take_loot_table(key, &tag, &mut entity);
                        self.world.block_entities.insert(key, entity);
                        self.world.modified.insert((pos.x >> 4, pos.z >> 4));
                    }
                }
                // A prime the mods refused is silent: no fuse sound, no game event.
                LevelEvent::Sound { pos, ref sound, .. }
                    if self.tnt_refused == Some(pos)
                        && &**sound == "minecraft:entity.tnt.primed" => {}
                LevelEvent::GameEvent {
                    pos,
                    event: "minecraft:prime_fuse",
                    ..
                } if self.tnt_refused == Some(pos) => self.tnt_refused = None,
                LevelEvent::Sound {
                    pos,
                    sound,
                    source,
                    volume,
                    pitch,
                } => {
                    let c = [
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ];
                    use sunrise_world::level::LevelSoundSource;
                    // Block placement carries a canonical key from its resolved sound type.
                    let qualified;
                    let name = if matches!(source, Some(LevelSoundSource::BlockPlacement { .. }))
                        || sound.contains(':')
                    {
                        &*sound
                    } else {
                        qualified = format!("minecraft:{sound}");
                        &qualified
                    };
                    if source.is_some() {
                        self.play_audio_on(
                            mod_api::SoundSource::Blocks.index(),
                            name,
                            Some(c),
                            volume,
                            pitch,
                        );
                        if let Some(LevelSoundSource::BlockPlacement {
                            except: Some(placer),
                        }) = source
                            && let Some(captured) = self
                                .audio
                                .capture
                                .as_mut()
                                .and_then(|sounds| sounds.last_mut())
                        {
                            captured.except_entity = Some(placer);
                        }
                    } else {
                        self.play_audio(name, Some(c), volume, pitch);
                    }
                }
                // Handled as the behaviour emits it (GameLevel::emit).
                LevelEvent::DropResources { pos, state } => {
                    self.with_level(|_, level, _| level.drop_resources_now(pos, state));
                }
                LevelEvent::FallingBlock {
                    pos,
                    state,
                    hurt_per_distance,
                } => {
                    let mut damage = sunrise_client::falling_block::Damage::falling(
                        &self.res.facts().get(state).name,
                    );
                    if let Some(per_distance) = hurt_per_distance {
                        damage.hurt_entities = true;
                        damage.per_distance = per_distance;
                    }
                    let p = Vec3::new(
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y),
                        f64::from(pos.z) + 0.5,
                    );
                    // Admit before returning to the next scheduled block tick.
                    // A join veto does not undo the source replacement.
                    let _ = self.spawn_falling_block_entity(state, p, Vec3::ZERO, true, damage);
                }
                LevelEvent::Player { pos, effect, .. } => match effect {
                    PlayerEffect::OpenMenu => self.open_block_effect_menu([pos.x, pos.y, pos.z]),
                    PlayerEffect::Sleep if self.beds_destroy_on_use() => {
                        self.destroy_bed_on_use([pos.x, pos.y, pos.z])?
                    }
                    PlayerEffect::Sleep => self.try_sleep([pos.x, pos.y, pos.z]),
                    PlayerEffect::GiveItem(item) => {
                        if let Some(rest) = self.add_stack_to_inventory(ItemStack {
                            item,
                            count: 1,
                            damage: 0,
                            extra: None,
                        }) {
                            self.pop_resource([pos.x, pos.y, pos.z], rest);
                        }
                    }
                    // Popped as the behaviour emits it (GameLevel::emit).
                    PlayerEffect::DropItem(_) => {}
                    PlayerEffect::EatCake => {
                        // CakeBlock.eat
                        self.award_custom_stat("eat_cake_slice", 1);
                        if let Some(p) = self.player.as_mut() {
                            let food = &mut p.health.food;
                            food.food = (food.food + 2).min(20);
                            food.saturation = (food.saturation + 0.4).min(food.food as f32);
                        }
                    }
                    PlayerEffect::RespawnAnchor => {
                        self.respawn_anchor_used([pos.x, pos.y, pos.z])?
                    }
                    PlayerEffect::InteractLoot(_) => {}
                },
                LevelEvent::PrimeTnt { pos, rot } => self.prime_tnt_event(pos, rot),
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "dispense",
                } => self.dispense_from(pos, state)?,
                LevelEvent::BlockEvent {
                    pos, kind: "golem", ..
                } => self.check_golem_spawn([pos.x, pos.y, pos.z])?,
                LevelEvent::BlockEvent {
                    pos,
                    kind: "command_block",
                    ..
                } => self.command_block_tick(pos),
                LevelEvent::BlockEvent {
                    pos,
                    kind: "structure_block",
                    ..
                } => self.structure_block_neighbor_changed((pos.x, pos.y, pos.z)),
                LevelEvent::BlockEvent {
                    pos,
                    kind: "test_block",
                    ..
                } => self.test_block_neighbor_changed((pos.x, pos.y, pos.z)),
                LevelEvent::BlockEvent {
                    pos,
                    kind: "test_block_tick",
                    ..
                } => self.reset_test_block((pos.x, pos.y, pos.z)),
                LevelEvent::BlockEvent {
                    pos,
                    kind: "brushable",
                    ..
                } => self.brushable_tick([pos.x, pos.y, pos.z]),
                LevelEvent::BlockEvent {
                    pos, kind: "hatch", ..
                } => {
                    // FrogspawnBlock.hatchFrogspawn: 2-5 tadpoles.
                    let at = [
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ];
                    self.play_audio("minecraft:block.frogspawn.hatch", Some(at), 1.0, 1.0);
                    let count = 2 + self.world.random.next_int_bound(4).unwrap_or(0);
                    for _ in 0..count {
                        // getRandomTadpolePositionOffset: 0.2..0.8 inside the block.
                        let r = &mut self.world.random;
                        let off = |r: &mut sunrise_core::random::LegacyRandom| {
                            f64::from(r.next_float()) * 0.6 + 0.2
                        };
                        let (dx, dz) = (off(r), off(r));
                        let spawn = [
                            f64::from(pos.x) + dx,
                            f64::from(pos.y) - 0.5,
                            f64::from(pos.z) + dz,
                        ];
                        let _ = self.spawn_mob(
                            "minecraft:tadpole",
                            spawn,
                            true,
                            creatures::SpawnSite::Plain,
                        );
                    }
                }
                LevelEvent::BlockEvent {
                    pos,
                    kind: "hatch_sniffer",
                    ..
                } => self.hatch_sniffer([pos.x, pos.y, pos.z]),
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "hatch_turtles",
                } => self.hatch_turtles([pos.x, pos.y, pos.z], state),
                LevelEvent::BlockEvent {
                    pos,
                    kind: "portal_spawn",
                    ..
                } => self.portal_spawn([pos.x, pos.y, pos.z]),
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "spawn_ghastling",
                } => self.spawn_ghastling([pos.x, pos.y, pos.z], state),
                LevelEvent::GameEvent { pos, event, state } => {
                    self.sculk_level_event(pos, event, state)
                }
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "shrieker_respond",
                } => self.shrieker_block_event(pos, state),
                LevelEvent::BlockEntityEvent { pos, a, b, .. } => {
                    self.block_entity_event([pos.x, pos.y, pos.z], a, b)
                }
                // NoteBlock.triggerEvent: a NOTE particle above, coloured by
                // the note (`note / 24.0` in the x speed).
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "note_particle",
                } => {
                    use sunrise_client::particles::{ParticleOptions, ParticleType};
                    let note = self
                        .pack
                        .assets
                        .registry
                        .state(state)
                        .and_then(|s| s.properties.get("note")?.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    self.particles.add(
                        &ParticleOptions::simple(ParticleType::Note),
                        [
                            f64::from(pos.x) + 0.5,
                            f64::from(pos.y) + 1.2,
                            f64::from(pos.z) + 0.5,
                        ],
                        [note / 24.0, 0.0, 0.0],
                    );
                }
                // NoteBlock.getCustomSoundId: the skull above's note block sound,
                // played at volume 3 (vanilla's pitch for a head is 1).
                LevelEvent::BlockEvent {
                    pos,
                    kind: "note_custom_sound",
                    ..
                } => {
                    let sound = match self.world.block_entities.get(&(pos.x, pos.y + 1, pos.z)) {
                        Some(BlockEntity::Skull(skull)) => skull.note_block_sound.clone(),
                        _ => None,
                    };
                    if let Some(sound) = sound.as_deref() {
                        let at = [
                            f64::from(pos.x) + 0.5,
                            f64::from(pos.y) + 0.5,
                            f64::from(pos.z) + 0.5,
                        ];
                        self.play_audio(sound, Some(at), 3.0, 1.0);
                    }
                }
                // `level.addParticle`: only the local player's own click draws it.
                LevelEvent::BlockEvent {
                    kind: "candle_smoke",
                    ..
                } if self.lan_acting.is_some() => {}
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "candle_smoke",
                } => self.candle_smoke([pos.x, pos.y, pos.z], state),
                // The block at `pos` has just replaced `state`: Block.pushEntitiesUp.
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "push_entities_up",
                } => {
                    self.push_entities_up(
                        pos,
                        state,
                        self.world.block(pos.x, pos.y, pos.z),
                        |_| true,
                    );
                }
                LevelEvent::GrowTree { .. }
                | LevelEvent::PlaceFeature { .. }
                | LevelEvent::BlockEvent { .. } => {}
            }
        }
        Ok(())
    }
    /// Menus opened by PlayerEffect::OpenMenu.
    fn open_block_effect_menu(&mut self, pos: [i32; 3]) {
        if self.fake_blocks(mod_api::FakeEffect::Menu) {
            return;
        }
        let name = self
            .res
            .facts()
            .get(self.world.block(pos[0], pos[1], pos[2]))
            .name
            .clone();
        let key = (pos[0], pos[1], pos[2]);
        if self.chest_menu_blocked(key, &name) {
            // The use still succeeds, but no menu, stat or piglin anger.
            return;
        }
        // BaseContainerBlockEntity.createMenu: a lock the opener cannot
        // unlock refuses the menu with vanilla's action bar and sound.
        if self.refuse_locked_container(key, &name) {
            return;
        }
        if &*name == "minecraft:lectern" {
            self.open_lectern(pos);
            return;
        }
        if sunrise_client::command_block::Mode::of_block(&name).is_some() {
            self.open_command_block(key);
            return;
        }
        // The game master blocks' screens open on the player's own client
        // (`BlockEntity.usedBy`); the server has nothing to open.
        if matches!(
            &*name,
            "minecraft:structure_block"
                | "minecraft:jigsaw"
                | "minecraft:test_block"
                | "minecraft:test_instance_block"
        ) {
            return;
        }
        // SignBlock.useWithoutItem: the text editor, not a container.
        let block = self.world.block(pos[0], pos[1], pos[2]);
        if self.res.facts().get(block).sign_form().is_some() {
            self.use_sign_empty_hand(pos);
            return;
        }
        if &*name == "minecraft:crafting_table" {
            self.craft_size = 3;
            self.craft_table = Some(pos);
        } else if &*name == "minecraft:ender_chest" {
            self.open_ender_chest(key);
            // EnderChestBlock.useWithoutItem
            self.award_custom_stat("open_enderchest", 1);
            self.anger_piglins(true);
            // Guests start the opener only when the handoff installs their menu.
            // EnderChestBlockEntity.startOpen: the first opener's CONTAINER_OPEN.
            if self.lan_acting.is_none() && self.container_opener_event(key, true) {
                self.post_block_game_event(
                    "minecraft:container_open",
                    [key.0, key.1, key.2],
                    Some(sunrise_client::mobs::Who::Player),
                    None,
                );
                self.container_edge_sound(key, true);
            }
        } else if matches!(&*name, "minecraft:beacon" | "minecraft:enchanting_table")
            && self.open_station(pos, &name)
        {
            // Station menus whose block has a block entity, not a container.
            self.world.block_entities.get_or_insert_with(key, || {
                BlockEntity::for_block(&name).unwrap_or(BlockEntity::Chest(Vec::new()))
            });
        } else if storage::is_shulker_box(&name) && !self.shulker_can_open(pos) {
            // ShulkerBoxBlock.canOpen: the lid has no room; nothing opens.
        } else if let Some(entity) = BlockEntity::for_block(&name) {
            self.world.block_entities.get_or_insert_with(key, || entity);
            sunrise_client::spans::time("open_loot", || self.unpack_opened_loot(key, &name));
            let centre = [
                f64::from(key.0) + 0.5,
                f64::from(key.1) + 0.5,
                f64::from(key.2) + 0.5,
            ];
            sunrise_client::spans::time("open_maps", || {
                self.resolve_container_maps(key, centre);
            });
            self.container = Some(key);
            self.open_container_slots = Some((key, self.open_container_size()));
            // ChestMenu and ShulkerBoxMenu start the opener in their constructors.
            // For a guest, open_block_menu does that after the turn's handoff.
            let first = self.lan_acting.is_none() && self.container_opener_event(key, true);
            if first {
                self.post_block_game_event(
                    "minecraft:container_open",
                    [key.0, key.1, key.2],
                    Some(sunrise_client::mobs::Who::Player),
                    None,
                );
            }
            let stat = match &*name {
                "minecraft:chest" => "open_chest",
                "minecraft:trapped_chest" => "trigger_trapped_chest",
                "minecraft:barrel" => "open_barrel",
                "minecraft:hopper" => "inspect_hopper",
                "minecraft:dropper" => "inspect_dropper",
                "minecraft:dispenser" => "inspect_dispenser",
                n if storage::is_shulker_box(n) => "open_shulker_box",
                _ => "",
            };
            if !stat.is_empty() {
                self.award_custom_stat(stat, 1);
            }
            // Opening a piglin-guarded container angers the piglins who see it.
            if self.guarded_by_piglins(&name) {
                self.anger_piglins(true);
            }
            if first {
                self.container_edge_sound(key, true);
            }
        } else if self.open_mod_menu(key) {
            // A mod block's container (`native_block_entity_container`).
            self.container = Some(key);
            if self.lan_acting.is_none() {
                self.mod_container_player_opener(key, true);
            }
        } else if self.open_station(pos, &name) {
            // Workstation menus: game/blocks/stations.rs.
        } else {
            self.message = Some((
                format!(
                    "{} is not available yet",
                    name.trim_start_matches("minecraft:").replace('_', " ")
                ),
                Instant::now(),
            ));
            return;
        }
        self.set_screen(Screen::Inventory);
        self.capture(false);
        self.set_mapping_held("key.use", false);
    }
    /// `DecoratedPotBlock.playerWillDestroy`: a pot mined with an item in
    /// `breaks_decorated_pots`, unless an enchantment prevents shattering, is
    /// cracked first (flags 260), so it shatters into its sherds.
    fn crack_mined_pot(&mut self, pos: [i32; 3], state: BlockStateId) -> BlockStateId {
        let registry = &self.pack.assets.registry;
        let Some(mut pot) = registry
            .state(state)
            .filter(|s| s.name == "minecraft:decorated_pot")
            .cloned()
        else {
            return state;
        };
        let Some(held) = self.held() else {
            return state;
        };
        let breaks = self
            .res
            .items()
            .item_tags
            .get("minecraft:breaks_decorated_pots")
            .is_some_and(|t| t.contains(&*held.item));
        let enchantments = &self.stations().enchantments;
        // `EnchantmentHelper.hasTag`.
        let kept = held
            .all_enchantments()
            .iter()
            .any(|(e, _)| enchantments.in_tag("minecraft:prevents_decorated_pot_shattering", e));
        if !breaks || kept {
            return state;
        }
        pot.properties.insert("cracked".into(), "true".into());
        let Ok(cracked) = registry.resolve(&pot) else {
            return state;
        };
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| b.set_block(level, ticks, at, cracked, 260));
        cracked
    }
    /// Marks every enclosing player break at `pos` in the current dimension as
    /// already removed so it does not repeat removal effects when it resumes.
    pub(in crate::game) fn invalidate_active_player_breaks(&mut self, pos: [i32; 3]) {
        let dim = self.dim;
        for active in &mut self.active_player_breaks {
            if active.dim == dim && active.pos == pos {
                active.removed_by_nested = true;
            }
        }
    }
    /// `CommonHooks.fireBlockBreak` where `ServerPlayerGameMode.destroyBlock` asks it: the
    /// acting player's checks, then the mods' `break_block`. The state at `pos` and the
    /// experience `break_block` left, or None when a check refused the break or a mod canceled it.
    pub(in crate::game) fn break_block_allowed(
        &mut self,
        pos: [i32; 3],
    ) -> Result<Option<(BlockStateId, Option<u32>)>> {
        // ServerPlayerGameMode.destroyBlock and CommonHooks.fireBlockBreak read the main hand,
        // whichever hand is acting.
        self.with_hand(Hand::Main, |game| {
            let state = game.world.block(pos[0], pos[1], pos[2]);
            let mut canceled = !game.debug_stick_can_destroy(pos)?
                || !game
                    .res
                    .items()
                    .can_destroy_block(game.held(), game.creative)
                || game.block_action_restricted(pos)
                || (!game.can_use_game_master_blocks()
                    && is_game_master_block(&game.res.facts().get(state).name));
            let mut exp_to_drop = None;
            if game.native_authoritative()
                && (!canceled || mod_api::__private::listened(mod_api::HookId::BreakBlock))
            {
                game.native_break_block(pos, state, &mut canceled, &mut exp_to_drop);
            }
            if canceled
                || (!game.can_use_game_master_blocks()
                    && is_game_master_block(&game.res.facts().get(state).name))
                || game.block_action_restricted(pos)
            {
                return Ok(None);
            }
            Ok(Some((state, exp_to_drop)))
        })
    }
    fn break_block_as_player(
        &mut self,
        pos: [i32; 3],
        collect: Option<&mut Vec<ItemStack>>,
    ) -> Result<bool> {
        let [x, y, z] = pos;
        // `Level.getBlockState` at the boundary: a column the level does not hold resident is
        // loaded to read it, and what it holds, air included, is the subject of the break
        // callback (`ServerPlayerGameMode.destroyBlock` has no air filter).
        if !self.world.is_loaded_at(x, y, z)
            && !(self.native_authoritative()
                && self.load_full_chunk(None, (x >> 4, z >> 4)).is_ok())
        {
            return Ok(false);
        }
        let state = self.world.block(x, y, z);
        // NeoForge CommonHooks.fireBlockBreak: pre-cancel when the held item,
        // game mode, or game-master check refuses the break, then post
        // `break_block` before applying `ServerPlayerGameMode.destroyBlock`'s
        // hard restrictions.
        let mut canceled = !self.debug_stick_can_destroy(pos)?
            || !self
                .res
                .items()
                .can_destroy_block(self.held(), self.creative)
            || self.block_action_restricted(pos)
            || (!self.can_use_game_master_blocks()
                && is_game_master_block(&self.res.facts().get(state).name));
        let mut exp_to_drop = None;
        // A break of this same block from inside the callback still outstanding on it does not
        // dispatch again: it would call itself without end. It still removes the block, as a nested
        // removal, and the outer break then applies none of its own.
        let outstanding = self
            .active_player_breaks
            .iter()
            .any(|active| active.dim == self.dim && active.pos == pos);
        let removed_by_nested = if self.native_authoritative()
            && !outstanding
            && (!canceled || mod_api::__private::listened(mod_api::HookId::BreakBlock))
        {
            let depth = self.active_player_breaks.len();
            self.active_player_breaks.push(ActivePlayerBreak {
                dim: self.dim,
                pos,
                removed_by_nested: false,
            });
            let guard = ActiveBreakGuard { game: self, depth };
            guard
                .game
                .native_break_block(pos, state, &mut canceled, &mut exp_to_drop);
            guard.game.active_player_breaks[depth].removed_by_nested
        } else {
            false
        };
        // If canceled, if a nested break already completed removal at this
        // position in this level (even when a block or fluid remains), or if
        // the listener left air here, do not apply removal effects again.
        if canceled || removed_by_nested || self.res.behaviours().is_air(self.world.block(x, y, z))
        {
            return Ok(false);
        }
        // ServerPlayerGameMode.destroyBlock: hard game-master and game-mode
        // restrictions still hold even if a `receive_cancelled` listener
        // cleared the pre-cancel.
        if (!self.can_use_game_master_blocks()
            && is_game_master_block(&self.res.facts().get(state).name))
            || self.block_action_restricted(pos)
        {
            return Ok(false);
        }
        self.input_feedback(sunrise_client::input_source::Feedback::BlockBroken);
        let broken = state;
        let facts = self.res.facts().get(state).clone();
        let key = (pos[0], pos[1], pos[2]);
        self.unpack_loot(container_loot::LootTarget::Block(key), None);
        let live = (!self.creative)
            .then(|| self.live_block_entity(key, &facts.name))
            .flatten();
        let mut saved_data = None;
        // Mod data is encoded only if the removed block goes on to roll loot.
        let mod_source = match &live {
            Some((BlockEntity::Mod(_), _)) => Some((
                self.world.block_entities.generation(&key),
                self.world.block_names.get(&key).cloned(),
                self.world.block_lock(key).cloned(),
            )),
            Some((entity, _)) => {
                let saved = sunrise_client::save::block_entity_encode::encode(
                    key,
                    entity,
                    &facts.name,
                    self.res.items(),
                );
                self.pending.record_from(&self.world, pos, saved);
                None
            }
            None => {
                if !self.creative
                    && let Some(tag) = spill::saved_entity(&self.world, pos)
                {
                    self.pending.record_saved(pos, &tag);
                    // Pots and other saved-only entities have no resident object, but
                    // this is still the original entity the break is about to remove.
                    if self.native_authoritative()
                        && mod_api::__private::listened(mod_api::HookId::BlockDrops)
                    {
                        saved_data = spill::packed_block_entity_payload(tag);
                    }
                }
                None
            }
        };
        let hive = match self.world.block_entities.get(&key) {
            Some(BlockEntity::Beehive(h)) => self.hive_state(pos).map(|s| ((**h).clone(), s)),
            _ => None,
        };
        let filled_hive = hive
            .as_ref()
            .is_some_and(|(h, (honey, _))| !h.bees.is_empty() || honey.is_some_and(|l| l > 0));
        let creative = self.creative;
        let player_destroy::PlayerRemoval {
            removed,
            state,
            harvest,
            tool: held,
            broken_tool,
        } = self.player_destroy_block_removal(pos, state, &facts, filled_hive);
        if !creative {
            if removed && harvest {
                // Block.playerDestroy: the mined statistic, the exhaustion and the drops.
                self.award_stat("mined", &facts.name, 1);
                if let Some(p) = self.player.as_mut() {
                    p.cause_food_exhaustion(0.005);
                }
                if let (Some((entity, _)), Some((generation, name, lock))) = (&live, mod_source) {
                    let saved = self.encode_removed_block_entity(
                        key,
                        entity,
                        generation,
                        &facts.name,
                        self.res.items(),
                    );
                    self.pending
                        .record_source(pos, spill::source_snapshot(saved, name, lock.as_ref()));
                }
                let stacks = self.block_loot_at(pos, state, held.as_ref());
                let silk = held
                    .as_ref()
                    .is_some_and(|h| h.enchantment_level("minecraft:silk_touch") > 0);
                // Block.dropResources: the item entities, then spawnAfterBreak's
                // experience, with the mods' say between the two.
                let breaker = Some(self.native_acting_id());
                let block_entity_data = live
                    .as_ref()
                    .and_then(|(_, data)| data.as_ref())
                    .or(saved_data.as_ref());
                let dropped = self.player_block_dropped(
                    (pos, state),
                    (breaker, held.as_ref(), block_entity_data),
                    (stacks, exp_to_drop),
                );
                let canceled = dropped.canceled;
                if let Some(kept) = collect {
                    kept.extend(dropped.items.into_iter().map(|item| item.stack));
                    if dropped.experience > 0 {
                        self.spawn_orbs(pos.map(|v| f64::from(v) + 0.5), dropped.experience);
                    }
                } else {
                    self.spawn_block_drops(pos, dropped);
                }
                // BeehiveBlock.playerDestroy: silk touch keeps the bees in, so
                // the trigger counts them; otherwise they are out already.
                if let Some((hive, (honey, facing))) = hive {
                    let bees = if silk { hive.bees.len() as i64 } else { 0 };
                    if !silk {
                        self.hive_broken_by_player(pos, hive, honey, facing);
                    }
                    self.trigger_bee_nest_destroyed(broken, held.as_ref(), bees);
                }
                // InfestedBlock.spawnAfterBreak: its silverfish comes out unless the
                // tool has silk touch (#prevents_infested_spawns) or blocks don't drop.
                if facts.name.starts_with("minecraft:infested_")
                    && !silk
                    && !canceled
                    && self
                        .ui
                        .rules
                        .bool_at(sunrise_client::gamerules::vanilla::BLOCK_DROPS)
                {
                    let at = [
                        f64::from(pos[0]) + 0.5,
                        f64::from(pos[1]),
                        f64::from(pos[2]) + 0.5,
                    ];
                    if let Some(id) = self.spawn_mob(
                        "minecraft:silverfish",
                        at,
                        false,
                        creatures::SpawnSite::Plain,
                    ) {
                        self.mob_poof(id);
                    }
                }
                // TurtleEggBlock.playerDestroy: after the drops, one egg of a
                // mined stack is taken and the rest stay.
                sunrise_threads::work::clear_work();
                self.with_level(|b, level, ticks| {
                    b.turtle_egg_destroyed_by_player(
                        level,
                        ticks,
                        BlockPos::new(pos[0], pos[1], pos[2]),
                        broken,
                    );
                });
            }
            // IceBlock.playerDestroy: the ice melts back into water (the
            // #prevents_ice_melting tool tag holds only silk touch).
            let prevents_melting = held
                .as_ref()
                .is_some_and(|h| h.enchantment_level("minecraft:silk_touch") > 0);
            if removed {
                sunrise_threads::work::clear_work();
                self.with_level(|b, level, ticks| {
                    b.ice_destroyed_by_player(
                        level,
                        ticks,
                        BlockPos::new(pos[0], pos[1], pos[2]),
                        broken,
                        prevents_melting,
                    );
                });
            }
            if let Some(original) = broken_tool {
                self.player_item_destroyed(&original, Some(Hand::Main));
            }
        }
        self.pending.clear(pos);
        self.process_level_events()?;
        Ok(removed)
    }
    /// The `ItemStack.interactLivingEntity` fallback in `Player.interactOn`,
    /// currently for native Mob entries after the entity's own action passed.
    fn item_interact_mob(&mut self, target: Option<InteractTarget>) -> bool {
        if self.spectator() || !mod_api::__private::has_part_hook(mod_api::HookId::ItemInteractMob)
        {
            return false;
        }
        let Some(index) = self.mob_interaction_target(target) else {
            return false;
        };
        let mob = &self.mobs[index];
        if mob.removed || !mob.alive() {
            return false;
        }
        let (entity, kind) = (
            mob.id as i32,
            sunrise_client::command::args::normalize(&mob.kind.id),
        );
        let Some(stack) = self.held().filter(|stack| stack.count > 0).cloned() else {
            return false;
        };
        let mut result = None;
        self.native_item_interact_mob(self.hand, entity, &kind, &stack, &mut result);
        result.is_some_and(|answer| self.interaction_consumed(answer))
    }
    /// `ItemStack.applyAfterUseComponentSideEffects`: a used stack with a
    /// `use_cooldown` component starts its entry (`UseCooldown.apply`). A
    /// server's guest also gets `ClientboundCooldownPacket`
    /// (`ServerItemCooldowns.onCooldownStarted`), queued for their turn's end.
    pub(super) fn apply_use_cooldown(&mut self, stack: &ItemStack) {
        let default = self
            .res
            .items()
            .get(&stack.item)
            .and_then(|i| i.use_cooldown.clone());
        let Some(component) = sunrise_client::cooldowns::component(stack, default.as_ref()) else {
            return;
        };
        let group = sunrise_client::cooldowns::group(stack, default.as_ref());
        let mut cancel = false;
        if let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone()) {
            self.native_player_item_cooldown(
                player,
                &name,
                &stack.item,
                component.ticks(),
                &mut cancel,
            );
        }
        if !cancel {
            self.add_cooldown(group, component.ticks());
        } else if self.lan_acting.is_some() {
            // The guest's client started the cooldown itself; 0 ends it there.
            self.guest_cooldowns.push((group, 0));
        }
    }
    /// `ItemCooldowns.addCooldown` (a guest also gets the packet).
    pub(super) fn add_cooldown(&mut self, group: Arc<str>, ticks: i32) {
        self.cooldowns.add(Arc::clone(&group), ticks);
        if self.lan_acting.is_some() {
            self.guest_cooldowns.push((group, ticks));
        }
    }
    fn tick_mining(&mut self) -> Result<()> {
        if self.destroy_delay > 0 {
            self.destroy_delay -= 1;
        }
        if !self.mapped_key_down("key.attack")
            || self.screen != Screen::Playing
            || self.dialog_open()
        {
            self.mining = None;
            return Ok(());
        }
        let Some(hit) = self.pick() else {
            self.mining = None;
            return Ok(());
        };
        // Player.blockActionRestricted: spectators, and adventure players
        // without a matching can_break item.
        if self.block_action_restricted(hit.pos)
            || !self
                .res
                .items()
                .can_destroy_block(self.held(), self.creative)
        {
            self.mining = None;
            return Ok(());
        }
        if self.creative {
            if self.destroy_delay == 0 {
                self.break_block(hit.pos)?;
                self.destroy_delay = 5;
            }
            return Ok(());
        }
        if self.destroy_delay > 0 {
            return Ok(());
        }
        if self.mining.as_ref().is_none_or(|m| m.pos != hit.pos) {
            self.attack_block(hit.pos)?;
            self.mining = Some(Mining {
                pos: hit.pos,
                progress: 0.0,
                per_tick: 0.0,
                ticks: 0,
                crack: None,
            });
        }
        // BlockState.getDestroyProgress each tick: a block Part may answer for an unbreakable block.
        let progress = self.destroy_progress(hit.pos);
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        if progress == 0.0 && self.res.facts().get(state).hardness < 0.0 {
            // The server still counts its destroy ticks on an unbreakable block.
            let m = self.mining.as_mut().ok_or("mining")?;
            m.per_tick = 0.0;
            m.ticks += 1;
            let (pos, ticks) = (m.pos, m.ticks);
            self.destroy_progress_event(pos, hit.face, ticks);
            return Ok(());
        }
        self.swing_arm();
        let m = self.mining.as_mut().ok_or("mining")?;
        m.per_tick = progress;
        m.progress += progress;
        m.ticks += 1;
        // An instant break at START sends no destroy events.
        if m.progress < 1.0 || m.ticks > 1 {
            let (pos, ticks) = (m.pos, m.ticks);
            self.destroy_progress_event(pos, hit.face, ticks);
        }
        let m = self.mining.as_mut().ok_or("mining")?;
        // ServerPlayerGameMode breaks a player's block at once only when it is instant (START's
        // insta-mine); otherwise it waits for the client's STOP_DESTROY_BLOCK
        // (`guest_stop_digging`), never breaking it on its own count. Breaking it here would
        // take the block from under the player's crack before the player's client predicts the
        // break, so it gets neither particles nor sound.
        if m.progress >= 1.0 && (self.lan_acting.is_none() || m.ticks == 1) {
            let pos = m.pos;
            self.mining = None;
            self.break_block(pos)?;
            self.destroy_delay = 5;
        }
        self.update_local_crack();
        Ok(())
    }
    /// A column whose surface is a full solid, non-fluid, non-leaf block open to the sky.
    fn land_column(&self, x: i32, z: i32) -> Option<i32> {
        if self.dim == Dim::Nether {
            // Below the bedrock ceiling: a solid floor with two air blocks above.
            for y in (32..120).rev() {
                let floor = self.res.facts().get(self.world.block(x, y - 1, z));
                if floor.full_collision
                    && floor.fluid == Fluid::None
                    && self.res.facts().is_air(self.world.block(x, y, z))
                    && self.res.facts().is_air(self.world.block(x, y + 1, z))
                {
                    return Some(y);
                }
            }
            return None;
        }
        let mut y = self.world.top_y() - 1;
        while y > self.world.bottom_y() {
            let s = self.world.block(x, y, z);
            if !self.res.facts().is_air(s) {
                let f = self.res.facts().get(s);
                let ok = f.full_collision
                    && f.fluid == Fluid::None
                    && !f.name.contains("leaves")
                    && f.hardness >= 0.0;
                return ok.then_some(y + 1);
            }
            y -= 1;
        }
        None
    }
    /// Landing rules (farmland trampling, turtle eggs) wrote or removed the block
    /// raw: finish `Level.setBlock`'s neighbour updates, so a crop on trampled
    /// farmland breaks. The local player's and every server player's landings.
    /// The level events they send go out too: a turtle egg's break particles
    /// and sound (`decreaseEggs`' 2001, `Level.destroyBlock`'s for the last).
    ///
    /// A mod hears the change first (`native_entity_change_block_pre`, Bukkit's
    /// `EntityChangeBlockEvent`): cancelling puts the block back and skips the
    /// updates and events. `FarmlandBlock.fallOn` is decided separately by
    /// `trample_farmland` before it writes.
    ///
    /// The game event the rule posts comes back for the caller to post with the
    /// landing player in place (`post_landing_vibration`).
    fn landing_block_updates(
        &mut self,
        player: Option<u128>,
        event: &sunrise_client::landing::FallEvent,
    ) -> Option<LandingVibration> {
        use sunrise_client::landing::FallEvent;
        use sunrise_world::block_landing::LandingEvent;
        let mut vibration = None;
        // The block the landing changed, what it replaced, and what it became.
        let (position, from, to, flags) = match *event {
            FallEvent::Write {
                position,
                from,
                state,
                flags,
            } => (position, from, state, flags as i32),
            FallEvent::Destroy {
                position, state, ..
            } => {
                // `Level.destroyBlock`: BLOCK_DESTROY with no source, after the
                // write (`TurtleEggBlock.decreaseEggs`' last egg).
                vibration = Some(LandingVibration {
                    event: "minecraft:block_destroy",
                    position,
                    state,
                    from_entity: false,
                });
                (position, state, self.air_state(), level_flags::UPDATE_ALL)
            }
            // `FarmlandBlock.turnToBaseBlock`'s BLOCK_CHANGE from the entity that
            // trampled it, and `TurtleEggBlock.decreaseEggs`' BLOCK_DESTROY
            // (`Context.of(state)`, no source): the write is already applied.
            FallEvent::BlockEvent {
                kind,
                position,
                state,
                with_entity,
            } => {
                // A cancelled change put the block back: nothing happened.
                let current = self.world.block(position.x, position.y, position.z);
                let cancelled = match kind {
                    LandingEvent::BlockChange => current != state,
                    LandingEvent::BlockDestroy => current == state,
                };
                if cancelled {
                    return None;
                }
                return Some(LandingVibration {
                    event: match kind {
                        LandingEvent::BlockChange => "minecraft:block_change",
                        LandingEvent::BlockDestroy => "minecraft:block_destroy",
                    },
                    position,
                    state,
                    from_entity: matches!(kind, LandingEvent::BlockChange) && with_entity,
                });
            }
            FallEvent::LevelEvent {
                event,
                position,
                data,
            } => {
                self.level_event_at(event, [position.x, position.y, position.z], data);
                return None;
            }
            _ => return None,
        };
        if self.landing_change_cancelled(player, position, from, to) {
            self.world
                .set(position.x, position.y, position.z, from)
                .ok();
            return None;
        }
        if matches!(event, FallEvent::Destroy { .. }) {
            self.level_event_at(2001, [position.x, position.y, position.z], from.0 as i32);
        }
        self.with_level(|b, level, ticks| b.notify_written(level, ticks, position, flags));
        vibration
    }
    /// Post a landing's block game event (`FarmlandBlock.turnToBaseBlock`'s
    /// BLOCK_CHANGE, `TurtleEggBlock.decreaseEggs`' BLOCK_DESTROY) as the player
    /// whose landing it was, who must be in place (`as_guest`). A sculk sensor, a
    /// calibrated sensor and a warden hear it.
    fn post_landing_vibration(&mut self, vibration: LandingVibration) {
        let at = [
            vibration.position.x,
            vibration.position.y,
            vibration.position.z,
        ];
        let who = vibration
            .from_entity
            .then_some(sunrise_client::mobs::Who::Player);
        self.post_block_game_event(vibration.event, at, who, Some(vibration.state));
    }
    /// `Blocks.AIR.defaultBlockState`: what a landing's destroy leaves.
    fn air_state(&self) -> BlockStateId {
        self.pack
            .assets
            .registry
            .default_state("minecraft:air")
            .unwrap_or(BlockStateId(0))
    }
    /// Whether a mod cancelled a generic landing write (not `FarmlandBlock.fallOn`)
    /// (`native_entity_change_block_pre`): asked with the two blocks' names and
    /// the player whose landing it was (`None` when no player did it).
    fn landing_change_cancelled(
        &mut self,
        player: Option<u128>,
        position: BlockPos,
        from: BlockStateId,
        to: BlockStateId,
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::EntityChangeBlockPre) {
            return false;
        }
        let mut cancel = false;
        self.native_entity_change_block_pre(native_hooks::EntityChangeBlockPreCall {
            player,
            entity: None,
            kind: "minecraft:player",
            pos: [position.x, position.y, position.z],
            from,
            to,
            cancel: &mut cancel,
        });
        cancel
    }
    fn tick(&mut self) -> Result<()> {
        // Held through the carry too: a carried body's move asks the parts as its own does.
        let _parts = (!self.is_logical_server())
            .then(|| self.collision_parts())
            .flatten();
        // A client's tick of the level it shows holds the colliders as it began, then carries
        // its own player with the one it stands on; a server's level tick holds its own scope.
        let _scope = (!self.is_logical_server()).then(|| {
            sunrise_client_render::moving_colliders::Scope::enter_level(
                self.collider_level(),
                self.placed_moving_colliders(),
            )
        });
        self.tick_body()
    }
    fn tick_body(&mut self) -> Result<()> {
        self.expire_client_cracks();
        if self.agent.hold_server {
            return Ok(());
        }
        self.abort_open_transactions();
        self.update_interaction_ranges();
        self.tick_client_lead()?;
        if self.remote.is_some() {
            return self.with_residency_tick(Self::tick_remote);
        }
        let running = self.clock.world_runs();
        if running {
            sunrise_client::spans::time("tick_functions", || self.tick_functions());
        }
        self.tick_first_level(running)
    }
    /// The title remains the no-level sentinel while debug options or a mod's
    /// screen covers it. Only UI dispatch ignores it; world loading and
    /// simulation still see it.
    fn title_input_active(&self) -> bool {
        self.title.is_some() && !matches!(self.screen, Screen::DebugOptions | Screen::Mod)
    }
    #[cfg(test)]
    fn hud(&mut self, size: [u32; 2]) -> HudBatch {
        self.hud_with_reload(size, None)
    }

    /// Aligns `locator_camera` with this frame's render camera before HUD extraction
    /// (`GameRenderer.renderLevel` sets `mainCamera` before `Gui.render`).
    pub(in crate::game) fn extract_frame_hud(
        &mut self,
        size: [u32; 2],
        reload_overlay: Option<(f32, sunrise_client::level_loading::OverlayFade)>,
        title: bool,
        camera: [f64; 3],
        view_yaw: f32,
        view_pitch: f32,
        fov_degrees: f32,
    ) -> HudBatch {
        self.locator_camera = (!title).then_some((camera, view_yaw, view_pitch, fov_degrees));
        self.hud_with_reload(size, reload_overlay)
    }

    fn hud_with_reload(
        &mut self,
        size: [u32; 2],
        reload_overlay: Option<(f32, sunrise_client::level_loading::OverlayFade)>,
    ) -> HudBatch {
        // Gui.extractRenderState passes integer coordinates to the HUD and screen. Keep
        // the projected fractions for input, but draw and hit-test hover at the frame's pixel.
        let pointer = self.project_window_cursor(size);
        if let Some(pointer) = pointer {
            self.cursor = pointer.pixel();
        }
        // Off a world (title, options, Win, a prompt before joining) the HUD shows nothing to
        // keep clear of; `hud_layers` fills it in once the world's HUD draws.
        let gui = HudBatch::new(size, self.settings.gui_scale_inputs());
        self.ui.room = sunrise_client::hud_kit::room::Room::new(gui.width, gui.height);
        // The pack prompt replaces the connecting screen (setScreen), but
        // in game it opens over the world.
        self.settle_title_toasts();
        let mut batch = if (self.server_packs.prompt.is_some()
            || self.code_of_conduct.is_some()
            || self.dialog_open())
            && self.player.is_none()
        {
            gui
        } else {
            self.hud_layers(size)
        };
        // Vanilla's confirmation replaces the dialog as the screen.
        if self.command_prompt.is_none() {
            self.draw_dialog(&mut batch);
        }
        self.draw_pack_prompt(&mut batch);
        self.draw_code_of_conduct(&mut batch);
        self.draw_link_prompt(&mut batch);
        self.draw_command_prompt(&mut batch);
        if let Some((bar, fade)) = reload_overlay {
            self.draw_reload_overlay(&mut batch, bar, fade);
        }
        // Gui.extractRenderState draws toasts over the active screen or overlay,
        // after the saving indicator and before the debug overlay.
        self.draw_every_toast(&mut batch);
        // Gui.extractRenderState: the debug overlay comes after the screen
        // and the toasts, so it is over everything, menus included.
        self.draw_debug_overlay(&mut batch, size);
        self.draw_deferred_subtitles(&mut batch);
        // The layers crates registered to draw over everything (`sunrise_hooks::hud`).
        sunrise_client::extras_hud::draw_top_layers(
            sunrise_hooks::hud::engine::all(),
            &mut batch,
            &self.gui.atlas,
        );
        self.draw_pad_over_hud(&mut batch);
        if let Some(pointer) = pointer {
            debug_assert_eq!(
                self.cursor,
                pointer.pixel(),
                "drawing changed the input cursor"
            );
            self.cursor = pointer.subpixel();
        }
        self.pack.item_renderer.end_gui_frame();
        batch
    }
    fn draw_reload_overlay(
        &self,
        batch: &mut HudBatch,
        bar: f32,
        fade: sunrise_client::level_loading::OverlayFade,
    ) {
        sunrise_client::level_loading::draw_loading_overlay(
            batch,
            &self.gui.atlas,
            bar,
            fade,
            self.settings.file.bool("darkMojangStudiosBackground"),
        );
    }

    fn hud_layers(&mut self, size: [u32; 2]) -> HudBatch {
        use sunrise_client::gui_layers::VanillaGuiLayer;

        self.input_hud_begin();
        self.subtitles.deferred.set(false);
        let mut batch = HudBatch::new(size, self.settings.gui_scale_inputs());
        batch.glint_colour = self.pvp_glint_texel();
        // GlobalSettingsUniform's GameTime, for a resource pack's text shader.
        batch.game_time = ((self.world.game_time.rem_euclid(24000)) as f32
            + self.clock.fraction() as f32)
            / 24000.0;
        let (w, h) = (batch.width, batch.height);
        // HDR calibration ends when its page does, whatever moved the screen
        // off Options (a death screen opened by a server, say).
        if self.screen != Screen::Options && self.ui.options.calibration_window.is_some() {
            self.show_calibration((w, h));
        }
        if self.screen == Screen::Win {
            let drawn = self.draw_screen_with_pre_hook(&mut batch);
            if drawn {
                self.draw_win_screen(&mut batch);
            }
            // The early return skips the match below: the hooks still see the
            // screen, after its own drawing.
            self.publish_screen_regions(&batch, None);
            self.draw_screen_with_post_hook(&mut batch);
            return batch;
        }
        if self.screen == Screen::Options && self.title.is_some() {
            let drawn = self.draw_screen_with_pre_hook(&mut batch);
            self.draw_options(&mut batch, drawn);
            self.publish_screen_regions(&batch, None);
            self.draw_screen_with_post_hook(&mut batch);
            return batch;
        }
        if let Some(title) = &mut self.title
            && std::mem::take(&mut title.onboarding_finished)
        {
            // Options.onboardingAccessibilityFinished.
            self.settings.file.set(
                "onboardAccessibility",
                sunrise_client::options_file::V::Bool(false),
            );
            self.settings.save(&Self::options_path());
        }
        if let Some(title) = &self.title {
            for found in title.lan_found_narrations((w, h)) {
                self.say_system_queued(&found);
            }
        }
        if matches!(self.screen, Screen::DebugOptions | Screen::Mod) && self.title.is_some() {
            // DebugOptionsScreen or a mod's screen replaces the title; no level
            // HUD is behind it.
            let drawn = self.draw_screen_with_pre_hook(&mut batch);
            if drawn && self.screen == Screen::DebugOptions {
                self.draw_debug_options(&mut batch);
            } else if drawn {
                self.draw_mod_screen(&mut batch);
            }
            self.publish_screen_regions(&batch, None);
            self.draw_screen_with_post_hook(&mut batch);
            return batch;
        }
        if self.title.is_some() {
            // TitleScreen.extractRenderState, then ToastManager over it: vanilla
            // draws the toast manager on every screen, so a pack download or a
            // resource-reload failure still shows on the title (#4428).
            // The buttons crates add show by what is installed now: the Mods
            // page's list changes while the title is open.
            let mods = self.settings.mods.installed.len();
            if let Some(title) = &mut self.title {
                title.mods = mods;
            }
            let drawn = self.draw_title_screen_with_pre_hook(&mut batch);
            if drawn && let Some(title) = &self.title {
                title.draw_with_items(
                    &self.gui.atlas,
                    Some(&self.pack.item_renderer),
                    &mut batch,
                    self.cursor,
                    self.settings.file.bool("hideSplashTexts"),
                    self.settings.file.bool("hideServerAddress"),
                    &mut |phase, batch| self.draw_screen_phase(batch, phase),
                );
                self.draw_preset_editor(&mut batch);
            }
            // ScreenEvent.Render.Post over the title's own screens: a mod draws
            // over them here (a pack browser over Create New World, say), and the
            // toasts still come after, as over any screen.
            self.publish_screen_regions(&batch, None);
            self.draw_title_screen_with_post_hook(&mut batch);
            return batch;
        }
        // The ToastManager draws over every screen: a server's packs
        // download while the connection screen shows.
        if self.remote_overlay(&mut batch) {
            self.publish_screen_regions(&batch, None);
            return batch;
        }
        // Hud.extractRenderState: F1 (keyToggleGui) hides camera overlays, the
        // crosshair, hotbar, effects, bars, sidebar, action bar, titles and chat;
        // the sleep overlay, debug screen and open screens still draw.
        let spectator = self.spectator();
        let camera = self.camera_player();
        let (wi, hi) = (w as i32, h as i32);
        let mut gui_canceled = false;
        let replaying = self.replay.playing();
        if !replaying {
            self.native_render_gui_pre(&mut gui_canceled);
        }
        // F1 hides vanilla drawing; RenderGuiEvent.Pre instead cancels the layer walk.
        let hide_hud = self.ui.hide_gui || replaying;
        let show_hud = !hide_hud && !gui_canceled;
        // A screen that keeps the in-game UI defers subtitles to its background,
        // even under F1. With no screen, Gui draws them after the debug overlay.
        let book_or_sign = self.book.is_some() || self.sign_edit.is_some();
        let subtitles = subtitles(
            show_hud,
            if self.server_packs.prompt.is_some() {
                Some(false)
            } else if self.screen != Screen::Playing || book_or_sign {
                Some(in_game_ui(self.screen, book_or_sign))
            } else {
                None
            },
        );
        let dispatch_hud = !gui_canceled && !replaying;
        self.hud_layer_next = dispatch_hud.then_some(0);
        self.hud_status_heights = Default::default();
        self.hud_status_heights_set.set(None);
        self.ui.room = self.hud_room(w, h, show_hud);
        let mut guide_from = 0;
        if dispatch_hud {
            if show_hud {
                self.draw_spyglass(&mut batch);
                guide_from = batch.vertices.len();
            }
            // Combat feel Legacy hides the attack indicator unless the player keeps it.
            let indicator = if show_hud && !self.pvp_hides_attack_indicator() {
                self.settings.attack_indicator
            } else {
                0
            };
            let crosshair_attack = (indicator == 1).then(|| {
                let speed = self.main_hand_attack_speed();
                (
                    self.hand_strength.scale(0.0, speed),
                    sunrise_client::attack_strength::delay(speed),
                )
            });
            self.gui_layer(VanillaGuiLayer::Crosshair, &mut batch, &mut |g, batch| {
                // F3's 3D crosshair takes the 2D one's place. A spectator only sees it over
                // something to open (`Hud.extractCrosshair`). These guards only gate vanilla paint.
                if show_hud
                    && g.camera_type.is_first_person()
                    && (!spectator || g.spectator_crosshair())
                    && !g.debug_crosshair()
                    && !g.overlay_holds_view()
                {
                    let full = crosshair_attack.is_some_and(|(strength, delay)| {
                        hud_layout::full_attack_indicator(strength, delay, g.crosshair_living())
                    });
                    let draws = hud_layout::crosshair(
                        wi,
                        hi,
                        crosshair_attack.map(|(strength, _)| strength),
                        full,
                    );
                    // The custom crosshair takes the sprite's place (the first draw); the
                    // attack indicator stays.
                    if g.custom_crosshair() {
                        g.draw_custom_crosshair(batch, wi, hi);
                        hud_layout::paint(batch, &g.gui.atlas, &draws[1..]);
                    } else {
                        hud_layout::paint(batch, &g.gui.atlas, &draws);
                    }
                }
            });
            self.gui_layer(VanillaGuiLayer::Hotbar, &mut batch, &mut |g, batch| {
                if !show_hud {
                    return;
                }
                if spectator {
                    g.draw_spectator_hotbar(batch, g.own_player_is_spectator());
                } else if let Some((inputs, source)) = g.camera_hotbar(camera) {
                    g.draw_item_hotbar(batch, [wi, hi], &inputs, source);
                    if indicator == 2 {
                        // Items and arm follow the camera; the charge belongs to the local player.
                        let mut progress = g.hand_strength.scale(0.0, g.main_hand_attack_speed());
                        g.native_hotbar_attack_indicator_progress(
                            native_hooks::HotbarAttackIndicatorProgressCall {
                                progress: &mut progress,
                            },
                        );
                        let mut draws = Vec::new();
                        hud_layout::hotbar_attack_indicator(
                            wi,
                            hi,
                            inputs.right_handed,
                            progress,
                            &mut draws,
                        );
                        hud_layout::paint(batch, &g.gui.atlas, &draws);
                    }
                }
            });
            self.gui_layer(VanillaGuiLayer::Effects, &mut batch, &mut |g, batch| {
                // The PvP & HUD effects widget takes the icons' place.
                if show_hud && !g.pvp_effects_shown() {
                    g.draw_effect_icons(batch);
                }
            });
            self.gui_layer(VanillaGuiLayer::BossOverlay, &mut batch, &mut |g, batch| {
                if show_hud {
                    g.draw_boss_bars(batch);
                }
            });
            if show_hud {
                self.draw_extras(&mut batch);
            }

            // NeoForge's Hud.registerVanillaLayers: each status row owns its own extraction.
            // The installed vanilla bodies share the column heights and a scratch draw list.
            let mut status_rows = Vec::new();
            let mount = if !show_hud {
                None
            } else {
                self.camera_vehicle_health(camera)
            };
            // NeoForge applies hudVisible and survivalVisible to the vanilla bodies only.
            let show_survival_status = show_hud && self.can_hurt_player();
            self.gui_layer(
                VanillaGuiLayer::PlayerHealth,
                &mut batch,
                &mut |g, batch| {
                    if !show_survival_status {
                        return;
                    }
                    let Some((inputs, kind)) = g.camera_health_inputs(camera) else {
                        return;
                    };
                    status_rows.clear();
                    g.health_hud.health_level(
                        &inputs,
                        kind,
                        wi,
                        hi,
                        &mut g.hud_status_heights,
                        &mut status_rows,
                    );
                    hud_layout::paint(batch, &g.gui.atlas, &status_rows);
                },
            );
            self.gui_layer(VanillaGuiLayer::ArmorLevel, &mut batch, &mut |g, batch| {
                if !show_survival_status {
                    return;
                }
                let Some(armor) = g.camera_armor(camera) else {
                    return;
                };
                status_rows.clear();
                hud_layout::armor_level(armor, wi, hi, &mut g.hud_status_heights, &mut status_rows);
                hud_layout::paint(batch, &g.gui.atlas, &status_rows);
            });
            self.gui_layer(VanillaGuiLayer::FoodLevel, &mut batch, &mut |g, batch| {
                if !show_survival_status || mount.is_some_and(|(hearts, _)| hearts != 0) {
                    return;
                }
                let Some(inputs) = g.camera_food_inputs(camera) else {
                    return;
                };
                status_rows.clear();
                g.health_hud.food_level(
                    &inputs,
                    wi,
                    hi,
                    &mut g.hud_status_heights,
                    &mut status_rows,
                );
                hud_layout::paint(batch, &g.gui.atlas, &status_rows);
            });
            self.gui_layer(
                VanillaGuiLayer::VehicleHealth,
                &mut batch,
                &mut |g, batch| {
                    let Some((hearts, health)) = mount else {
                        return;
                    };
                    status_rows.clear();
                    hud_layout::vehicle_health(
                        wi,
                        hi,
                        hearts,
                        health,
                        &mut g.hud_status_heights,
                        &mut status_rows,
                    );
                    hud_layout::paint(batch, &g.gui.atlas, &status_rows);
                },
            );
            self.gui_layer(VanillaGuiLayer::AirLevel, &mut batch, &mut |g, batch| {
                if !show_survival_status {
                    return;
                }
                let Some((inputs, sound_at)) = g.camera_air_inputs(camera) else {
                    return;
                };
                status_rows.clear();
                let pop_sound = g.health_hud.air_level(
                    &inputs,
                    wi,
                    hi,
                    &mut g.hud_status_heights,
                    &mut status_rows,
                );
                hud_layout::paint(batch, &g.gui.atlas, &status_rows);
                // Hud.playAirBubblePoppedSound: Player.playSound (PLAYERS), heard by this
                // client only, so another player's bubbles pop silently.
                if let Some((volume, pitch)) = pop_sound
                    && let Some(at) = sound_at
                {
                    let players = sunrise_client::settings::SOUND_SOURCES
                        .iter()
                        .position(|s| *s == "player")
                        .unwrap_or(0);
                    let capture = g.audio.capture.take();
                    g.play_audio_on(players, "ui.hud.bubble_pop", Some(at), volume, pitch);
                    g.audio.capture = capture;
                }
            });
            use sunrise_client::locator_bar::ContextualBar;
            // After the health rows (Hud.extractHotbarAndDecorations): jump, locator or XP bar.
            self.gui_layer(
                VanillaGuiLayer::ContextualInfoBar,
                &mut batch,
                &mut |g, batch| {
                    if !show_hud {
                        return;
                    }
                    let has_experience = g.can_hurt_player();
                    let contextual = g.contextual_bar();
                    if contextual == ContextualBar::JumpableVehicle {
                        g.draw_jump_bar(batch, wi, hi);
                    }
                    if contextual == ContextualBar::Locator {
                        let (sprite, [x, y, w, h]) =
                            sunrise_client::locator_bar::background(wi, hi);
                        sunrise_client::ui::nine_slice(
                            batch,
                            &g.gui.atlas,
                            sprite,
                            x as f32,
                            y as f32,
                            w as f32,
                            h as f32,
                            u32::MAX,
                        );
                    }
                    if has_experience && g.player.is_some() {
                        let atlas = &g.gui.atlas;
                        let width = |s: &str| HudBatch::text_width(atlas, s) as i32;
                        let draws = hud_layout::experience_colored(
                            g.pack.polytone_colors.xp_bar,
                            wi,
                            hi,
                            g.experience.progress,
                            g.experience.level,
                            contextual == ContextualBar::Experience,
                            &width,
                        );
                        hud_layout::paint(batch, atlas, &draws);
                    }
                    if contextual == ContextualBar::Locator {
                        let draws = g.locator_bar_draws(wi, hi);
                        hud_layout::paint(batch, &g.gui.atlas, &draws);
                    }
                },
            );
            self.gui_layer(
                VanillaGuiLayer::SelectedItemName,
                &mut batch,
                &mut |g, batch| {
                    if show_hud && !spectator {
                        g.draw_selected_name(batch, g.can_hurt_player());
                    }
                },
            );
            // Hud.extractSleepOverlay draws even under F1.
            self.gui_layer(
                VanillaGuiLayer::SleepOverlay,
                &mut batch,
                &mut |g, batch| {
                    let draws = hud_overlays::sleep_overlay(wi, hi, g.sleep_timer);
                    hud_layout::paint(batch, &g.gui.atlas, &draws);
                },
            );
            self.gui_layer(
                VanillaGuiLayer::OverlayMessage,
                &mut batch,
                &mut |g, batch| {
                    if show_hud {
                        g.draw_overlay_message(batch);
                    }
                },
            );
            self.gui_layer(VanillaGuiLayer::Title, &mut batch, &mut |g, batch| {
                if show_hud {
                    g.draw_title(batch);
                }
            });
            self.gui_layer(
                VanillaGuiLayer::ScoreboardSidebar,
                &mut batch,
                &mut |g, batch| {
                    if show_hud {
                        g.draw_scoreboard(batch);
                    }
                },
            );
            self.gui_layer(
                VanillaGuiLayer::SubtitleOverlay,
                &mut batch,
                &mut |g, batch| match subtitles {
                    Subtitles::Hidden => {}
                    Subtitles::Now => g.draw_subtitles(batch),
                    Subtitles::Deferred => g.subtitles.deferred.set(true),
                },
            );
            let atlas = &self.gui.atlas;
            if show_hud
                && let Some((text, at)) = &self.message
                && at.elapsed() < Duration::from_secs(3)
            {
                let tw = HudBatch::text_width(atlas, text);
                batch.text(atlas, text, (w - tw) / 2.0, h - 59.0, u32::MAX, true);
            }
            self.gui_layer(VanillaGuiLayer::Chat, &mut batch, &mut |g, batch| {
                // The chat screen has its own paint site for focused chat.
                if show_hud && g.screen != Screen::Chat && !g.overlay_holds_view() {
                    g.draw_chat(batch);
                }
            });
            self.gui_layer(VanillaGuiLayer::TabList, &mut batch, &mut |g, batch| {
                // Options polls world skins separately, even if the HUD is hidden.
                if show_hud && !matches!(g.screen, Screen::Chat | Screen::Options) {
                    g.draw_tab_list(batch);
                }
            });
            self.draw_mod_gui_layers(None, &mut batch);
            self.hud_layer_next = None;
            self.hud_status_heights_set.set(None);
            self.native_render_gui_post(&mut batch);
            self.hud_layer_bounds.clear();
        }
        // Screen.extractBlurredBackground blurs the world and HUD, while the
        // screen's background, widgets, toasts and debug overlay stay sharp.
        batch.blur_at = batch.vertices.len() as u32;
        // PackConfirmScreen takes the open screen's place (it comes back once
        // the prompt is answered): the screen under the prompt isn't drawn.
        let screen = if self.server_packs.prompt.is_some() || self.book.is_some() {
            Screen::Playing
        } else {
            self.screen
        };
        if screen == Screen::Options {
            self.flush_skin_slots();
            self.update_remote_skins();
        }
        let drawn = self.draw_screen_with_pre_hook(&mut batch);
        let mut region_screen = None;
        match screen {
            Screen::Options => self.draw_options(&mut batch, drawn),
            _ if !drawn => {}
            Screen::Paused if self.pause_menu => self.draw_pause_menu(&mut batch),
            // PauseScreen(false): only its title, at the top.
            Screen::Paused => {
                self.draw_screen_phase(
                    &mut batch,
                    sunrise_client::screen_draw::RenderPhase::Background,
                );
                let title = self.translate_with_fallback("menu.paused", "Game Paused");
                sunrise_client::widgets::begin_screen(|| {
                    sunrise_client::widgets::ScreenInfo::new("paused", title.clone())
                });
                let tw = HudBatch::text_width(&self.gui.atlas, &title);
                batch.text(
                    &self.gui.atlas,
                    &title,
                    ((batch.width / 2.0).floor() - (tw / 2.0).floor()).floor(),
                    10.0,
                    u32::MAX,
                    true,
                );
                self.draw_screen_phase(
                    &mut batch,
                    sunrise_client::screen_draw::RenderPhase::Foreground,
                );
            }
            Screen::GameModeSwitcher => self.draw_gamemode_switcher(&mut batch),
            Screen::Dead => self.draw_death_screen(&mut batch),
            Screen::Stats => self.draw_stats(&mut batch),
            Screen::Advancements => self.draw_advancements(&mut batch),
            Screen::Mod => self.draw_mod_screen(&mut batch),
            Screen::WorldOptions => self.draw_world_options(&mut batch),
            Screen::Friends => self.draw_friends_screen(&mut batch),
            Screen::Social => self.draw_social_screen(&mut batch),
            Screen::ServerLinks => self.draw_server_links(&mut batch),
            Screen::ConfirmLink => self.draw_confirm_link(&mut batch),
            Screen::DemoIntro => self.draw_demo_intro(&mut batch),
            Screen::DialogWarning => self.draw_dialog_warning(&mut batch),
            Screen::CommandBlock => self.draw_command_block(&mut batch),
            Screen::BlockEditor => self.draw_block_editor(&mut batch),
            Screen::DebugOptions => self.draw_debug_options(&mut batch),
            Screen::HudEditor => self.draw_hud_editor(&mut batch),
            Screen::Inventory => {
                let from = batch.vertices.len();
                if self.draw_inventory(&mut batch) {
                    region_screen = Some(Screen::Inventory);
                }
                self.draw_pad_menu_guide(&mut batch, from);
            }
            Screen::Creative => {
                let from = batch.vertices.len();
                if self.draw_creative(&mut batch) {
                    region_screen = Some(Screen::Creative);
                }
                self.draw_pad_menu_guide(&mut batch, from);
            }
            Screen::Chat => self.draw_chat_screen(&mut batch),
            Screen::Playing | Screen::Win => {}
        }
        // ScreenEvent.Render.Post: the open screen has drawn itself and its
        // tooltips, so a native mod draws over it here. Never `Playing`: the
        // prompts, books and sign editors drawn over gameplay keep it, draw
        // after this point and so over what a mod drew.
        self.publish_screen_regions(&batch, region_screen);
        self.draw_screen_with_post_hook(&mut batch);
        self.draw_saving_indicator(&mut batch);
        if self.server_packs.prompt.is_none() {
            self.remote_hud(&mut batch);
            self.draw_sign_editor(&mut batch);
            self.draw_book(&mut batch);
        }
        self.draw_studio_bars(&mut batch);
        if show_hud {
            self.draw_pad_guide(&mut batch, guide_from);
        }
        batch
    }

    pub(super) fn close_container(&mut self) {
        // `Gui.setScreen` clears the old screen's focus before it goes: the
        // key that closed the container (Tab bound to the inventory) has
        // already moved the focus, and it must not outlive the screen.
        self.pad_clear_focus();
        self.mod_browser_closed();
        self.reset_inventory_clicks();
        self.close_station();
        self.mount_closed();
        self.recipe_ui.reset_screen();
        if self.remote.is_some() {
            return self.remote_close_container();
        }
        let mut returned: Vec<ItemStack> = self.carried.take().into_iter().collect();
        returned.extend(self.craft_grid.iter_mut().filter_map(Option::take));
        for stack in returned {
            if let Some(rest) = self.add_stack_to_inventory(stack)
                && let Some(eye) = self.eye_position()
            {
                // AbstractContainerMenu.dropOrPlaceInInventory: `drop(stack, false)`.
                self.throw_from_eyes(rest, eye, false, false);
            }
        }
        self.craft_size = 2;
        self.craft_table = None;
        self.open_container_slots = None;
        if let Some(key) = self.container.take() {
            if self.lan_acting.is_none() {
                self.mod_container_player_opener(key, false);
            }
            // A cart's or chest boat's items go back to it.
            self.close_cart_container(key);
            let last = self.container_opener_event(key, false);
            if last {
                self.post_block_game_event(
                    "minecraft:container_close",
                    [key.0, key.1, key.2],
                    Some(sunrise_client::mobs::Who::Player),
                    None,
                );
                self.container_edge_sound(key, false);
            }
            self.world.modified.insert((key.0 >> 4, key.2 >> 4));
        }
        self.close_ender_chest();
    }

    /// `Camera.setRotation`: the camera's orientation, turned round in
    /// third-person front view.
    pub(super) fn camera_orientation(&self) -> glam::Quat {
        use sunrise_client::camera::CameraType::FirstPerson;
        match self.posed_rotation {
            // Extracted frame angles are already in the camera convention, including studio roll
            // and third-person front mirroring.
            Some((yaw, pitch, roll)) => camera_rotation_rolled(FirstPerson, yaw, pitch, roll),
            None if self.studio.flying() => camera_rotation_rolled(
                FirstPerson,
                self.studio.freecam.yaw,
                self.studio.freecam.pitch,
                -self.studio.freecam.roll,
            ),
            None => camera_rotation(self.camera_type, self.yaw, self.pitch),
        }
    }
    /// `mark_motion` ids of entities that aren't mobs or dropped items (boats, minecarts, armor
    /// stands, item frames, paintings, primed TNT, falling blocks, rockets, tridents, thrown
    /// potions): their entity id under this tag.
    pub(super) const MOTION_OBJECT: u64 = 3 << 32;
    /// An item frame's held item, apart from the frame (one id may mark one run).
    pub(super) const MOTION_FRAME_ITEM: u64 = 4 << 32;
    /// A block entity, by its position (`motion_block_entity`).
    pub(super) const MOTION_BLOCK_ENTITY: u64 = 5 << 32;
    /// A block a piston moves, by the cell it moves to (`motion_at`).
    pub(super) const MOTION_MOVING_BLOCK: u64 = 6 << 32;
    /// The player's own model (third person, and a shader pack's shadow pass).
    pub(super) const MOTION_OWN_PLAYER: u64 = 7 << 32;
    /// The local player's flame vertices are tracked separately from the body run.
    const MOTION_OWN_PLAYER_FIRE: u64 = Self::MOTION_OWN_PLAYER | 1;
    /// The entity whose geometry starts at vertex `start` (0: none), for frame
    /// generation to follow its own motion. Each run lasts until the next mark.
    pub(super) fn mark_motion(&self, start: usize, id: u64) {
        self.entity_motion.borrow_mut().push((start as u32, id));
        // Where its GPU-posed vertices start (`entity_skin`): a shader pack's `at_velocity`
        // follows those too.
        let posed = self.skin_frame.borrow().vertices;
        self.entity_motion_posed.borrow_mut().push(posed);
    }
    /// How far the block entity of a block state is drawn (its `BlockEntityRenderer`'s reach), and
    /// whether entity culling may hide it (the end portal and gateway never are), by state id,
    /// resolved once: every block entity draw and the frame's block entity selection ask.
    fn block_entity_reach_entry(&self, state: u32) -> (Reach, bool) {
        let table = self.block_entity_reach.get_or_init(|| {
            self.res
                .facts()
                .states
                .iter()
                .map(|state| {
                    let unculled = matches!(
                        &*state.name,
                        "minecraft:end_gateway" | "minecraft:end_portal"
                    );
                    (Reach::of(&state.name), unculled)
                })
                .collect()
        });
        // A state the registry lacks has air's facts (`BlockFacts::get`).
        table
            .get(state as usize)
            .copied()
            .unwrap_or((Reach::Default, false))
    }
    /// The reach of the block entity a block state holds (see [`Self::block_entity_reach_entry`]).
    pub(super) fn block_entity_reach(&self, state: u32) -> Reach {
        self.block_entity_reach_entry(state).0
    }
    /// ItemEntityRenderer: items bob and spin; blocks render as small cubes.
    fn entity_geometry(
        &mut self,
        camera: glam::DVec3,
        alpha: f64,
        billboard_frustum: Option<sunrise_client::gfx::JoinedFrusta>,
        mut out: Vec<Vertex>,
        additive: &mut Vec<Vertex>,
    ) -> (
        Vec<Vertex>,
        Arc<[StaticSectionSnapshot]>,
        Vec<[i32; 3]>,
        StaticCacheFrameStats,
    ) {
        // Its steps as spans (SUNRISE_SPIKES): each lap names the step that just ended.
        let mut laps = sunrise_client::spans::laps();
        let mut static_block_entities = std::sync::Arc::from([]);
        let mut visible_static_block_entity_sections = Vec::new();
        let mut static_block_entity_stats = StaticCacheFrameStats::default();
        self.entity_spans.borrow_mut().clear();
        self.entity_motion.borrow_mut().clear();
        self.entity_motion_posed.borrow_mut().clear();
        self.maps.slots.next_frame();
        // LevelExtractor.extract: the transient blocks queued since the last frame.
        self.drain_transient_block_queue();
        self.prepare_entity_culling();
        self.prepare_occlusion_culling(camera);
        self.prepare_trims();
        self.begin_skinning();
        self.mark_entity(out.len(), "minecraft:entity_flame");
        // One scan for both passes that draw them (block entities below and the
        // overlay near the end of this function).
        let cracks = self.cracks(camera);
        let fire_yaw = self.posed_rotation.map_or(self.view.y_rot, |(y, _, _)| y);
        // Packs may move vertices or draw from another camera. The guarded
        // billboard frustum covers TAA's jitter without changing model culling.
        let fire_view = billboard_frustum
            .as_ref()
            .filter(|_| self.pack_ids.is_none() && self.native_shaders.picked.is_empty());
        let fire_visible = |position, width, height| {
            fire_view.is_none_or(|view| {
                sunrise_client::fire_render::in_view(view, camera, position, width, height)
            })
        };
        laps.lap("eg_prep");
        for item in &self.dropped {
            if self.hides_as_camera(item.id) {
                continue;
            }
            if item.fire_ticks > 0
                && self
                    .stack_can_be_hurt_by(&item.stack, "minecraft:in_fire")
                    .unwrap_or(true)
            {
                let at = item.previous.lerp(item.position, alpha);
                if !fire_visible([at.x, at.y, at.z], 0.25, 0.25) {
                    continue;
                }
                sunrise_client::fire_render::flames(
                    [at.x, at.y, at.z],
                    0.25,
                    0.25,
                    camera,
                    fire_yaw,
                    &self.gui.atlas.sprites,
                    self.world.light(
                        at.x.floor() as i32,
                        at.y.floor() as i32,
                        at.z.floor() as i32,
                    ),
                    &mut out,
                );
            }
        }
        laps.lap("eg_fire_items");
        for stand in &self.armor_stands {
            if self.hides_as_camera(stand.id) {
                continue;
            }
            if stand.is_on_fire() && !stand.marker {
                let scale = if stand.small { 0.5 } else { 1.0 };
                let at = stand.render_position(alpha);
                if !fire_visible([at.x, at.y, at.z], 0.5 * scale, 1.975 * scale) {
                    continue;
                }
                sunrise_client::fire_render::flames(
                    [at.x, at.y, at.z],
                    0.5 * scale,
                    1.975 * scale,
                    camera,
                    fire_yaw,
                    &self.gui.atlas.sprites,
                    self.world.light(
                        at.x.floor() as i32,
                        at.y.floor() as i32,
                        at.z.floor() as i32,
                    ),
                    &mut out,
                );
            }
        }
        laps.lap("eg_fire_stands");
        // `LevelRenderer.submitFeatures`: `submitEntities` walks the entities in the order they
        // joined the level (`EntityLookup.byId`, as `LevelExtractor.extractVisibleEntities`
        // lists them), whatever the family, before `submitBlockEntities`. Each run of one
        // family is drawn by that family's own geometry.
        let orb_view = billboard_frustum
            .as_ref()
            .filter(|_| self.pack_ids.is_none() && self.native_shaders.picked.is_empty());
        // What a family sets up once a frame, not once per run: the first run of the family
        // builds it.
        let tnt_sprites = std::cell::OnceCell::new();
        let falling_setup = std::cell::OnceCell::new();
        let orb_frame = std::cell::OnceCell::new();
        let mut runs = std::mem::take(&mut self.entity_runs);
        let general_frame = self.begin_general_geometry(camera, alpha);
        let crystal_frame = self.begin_crystal_geometry();
        self.ordered_draw_runs(general_frame.is_some(), crystal_frame.is_some(), &mut runs);
        // Sets the CEM frame even if there are no mobs; later passes also use it.
        let mob_frame = self.begin_mob_geometry(camera, alpha);
        for run in &runs {
            use entity::DrawFamily as F;
            let range = run.range.clone();
            match run.family {
                F::Mob => self.mob_run_geometry(&mob_frame, range, &mut out),
                F::LocalPlayer => self.local_player_geometry(camera, &mut out),
                F::RemoteGeneral => {
                    if let Some(frame) = &general_frame {
                        self.general_run_geometry(frame, range, &mut out);
                    }
                }
                F::Dragon => {
                    // A dragon summoned outside the End draws at its join slot too.
                    if let Some(dragon) =
                        self.dragon.as_ref().filter(|d| !self.hides_as_camera(d.id))
                    {
                        self.dragon_geometry(dragon, camera, alpha, &mut out, additive);
                    }
                }
                F::Crystal => {
                    if let Some(frame) = &crystal_frame {
                        for crystal in &self.crystals[range] {
                            self.end_crystal_geometry(crystal, frame, camera, &mut out);
                        }
                    }
                }
                F::Item => {
                    // `ItemEntityRenderer`.
                    self.mark_entity(out.len(), "minecraft:item");
                    self.dropped_item_geometry(camera, alpha, &mut out, range);
                }
                F::Orb => {
                    self.mark_entity(out.len(), "minecraft:experience_orb");
                    let live = self.orbs[range]
                        .iter()
                        .filter(|o| !self.hides_as_camera(o.id))
                        .map(|o| {
                            let at = [
                                o.previous.x + (o.position.x - o.previous.x) * alpha,
                                o.previous.y + (o.position.y - o.previous.y) * alpha,
                                o.previous.z + (o.position.z - o.previous.z) * alpha,
                            ];
                            (o, at, alpha as f32)
                        });
                    let frame = orb_frame.get_or_init(|| self.orb_frame());
                    self.orb_geometry(camera, frame, live, orb_view, &mut out);
                }
                F::Falling => {
                    self.mark_entity(out.len(), "minecraft:falling_block");
                    let setup = falling_setup.get_or_init(|| self.falling_setup());
                    self.falling_geometry(camera, alpha, setup, range, &mut out);
                }
                F::Tnt => {
                    self.mark_entity(out.len(), "minecraft:tnt");
                    let sprites = *tnt_sprites.get_or_init(|| self.tnt_face_sprites());
                    self.tnt_geometry(camera, alpha, &mut out, range, sprites);
                }
                F::ArmorStand => {
                    self.mark_entity(out.len(), "minecraft:armor_stand");
                    self.armor_stand_geometry(camera, alpha, &mut out, range);
                }
                F::Boat => {
                    self.mark_entity(out.len(), "minecraft:oak_boat");
                    self.boat_geometry(camera, alpha, &mut out, range);
                }
                F::Cart => {
                    self.mark_entity(out.len(), "minecraft:minecart");
                    self.cart_geometry(camera, alpha, &mut out, range);
                }
                F::Trident => {
                    self.mark_entity(out.len(), "minecraft:trident");
                    self.trident_geometry(camera, alpha, &mut out, range);
                }
                F::Rocket => {
                    self.mark_entity(out.len(), "minecraft:firework_rocket");
                    self.rocket_geometry(camera, alpha, &mut out, range);
                }
            }
            self.mark_motion(out.len(), 0);
        }
        self.entity_runs = runs;
        laps.lap("eg_ordered_entities");
        // `VaultRenderer`.
        self.mark_entity(out.len(), "");
        self.vault_geometry(camera, alpha, &mut out);
        // `CampfireRenderer`.
        self.mark_entity(out.len(), "");
        self.campfire_geometry(camera, &mut out);
        self.brushable_geometry(camera, &mut out);
        self.shelf_geometry(camera, &mut out);
        self.mark_entity(out.len(), "minecraft:ominous_item_spawner");
        self.ominous_item_spawner_geometry(camera, alpha, &mut out);
        laps.lap("eg_items_misc");
        // Spawners and projectiles still have their own pass.
        self.other_creature_geometry(camera, alpha, &mut out);
        self.mark_motion(out.len(), 0);
        laps.lap("eg_other_creatures");
        // These billboards stay in their later pass, outside the ordered body runs.
        for m in &self.mobs {
            if self.hides_as_camera(m.id) {
                continue;
            }
            // Blaze.isOnFire: a charged blaze burns.
            let blazing = m.species() == sunrise_client::mobs::Species::Blaze && m.extra.flag;
            if (m.is_on_fire() || blazing) && !m.removed {
                let at = m.previous.lerp(m.position, alpha);
                if !fire_visible([at.x, at.y, at.z], m.width(), m.height()) {
                    continue;
                }
                self.mark_entity(out.len(), &m.kind.id);
                sunrise_client::fire_render::flames(
                    [at.x, at.y, at.z],
                    m.width(),
                    m.height(),
                    camera,
                    fire_yaw,
                    &self.gui.atlas.sprites,
                    self.world.light(
                        at.x.floor() as i32,
                        at.y.floor() as i32,
                        at.z.floor() as i32,
                    ),
                    &mut out,
                );
            }
        }
        if self.own_body_in_view()
            && let Some(p) = &self.player
            && p.is_on_fire()
            && !p.spectator
        {
            let feet = self
                .previous_position
                .lerp(p.position, self.clock.camera_fraction());
            if fire_visible(
                [feet.x, feet.y, feet.z],
                p.dimensions.width,
                p.dimensions.height,
            ) {
                self.mark_player(out.len(), false);
                self.mark_motion(out.len(), Self::MOTION_OWN_PLAYER_FIRE);
                sunrise_client::fire_render::flames(
                    [feet.x, feet.y, feet.z],
                    p.dimensions.width,
                    p.dimensions.height,
                    camera,
                    fire_yaw,
                    &self.gui.atlas.sprites,
                    self.world.light(
                        feet.x.floor() as i32,
                        feet.y.floor() as i32,
                        feet.z.floor() as i32,
                    ),
                    &mut out,
                );
                self.mark_motion(out.len(), 0);
            }
        }
        laps.lap("eg_fire_living");
        self.mark_entity(out.len(), "minecraft:evoker_fangs");
        self.fangs_geometry(camera, alpha, &mut out);
        self.block_item_display_geometry(camera, alpha, &mut out);
        laps.lap("eg_displays");
        let mut see_through = std::mem::take(&mut self.see_through_vertices);
        see_through.clear();
        let mut st_collector = std::mem::take(&mut self.see_through_collector);
        st_collector.clear();
        self.name_tag_geometry(camera, alpha, &mut out, &mut st_collector);
        self.test_instance_marker_geometry(camera, &mut out, &mut st_collector);
        self.debug_renderer_geometry(camera, &mut out);
        self.text_display_geometry(camera, alpha, &mut out, &mut st_collector);
        st_collector.flush(&mut see_through);
        self.see_through_collector = st_collector;
        self.see_through_vertices = see_through;
        laps.lap("eg_name_tags");
        // Entity shadows (EntityRenderer.extractShadow); a native pack's sun shadows replace them.
        self.mark_translucent_entity(out.len(), "minecraft:entity_shadow");
        let pack_shadows = self.renderer.as_ref().is_some_and(|r| {
            r.pack_entity_shadows() || (self.overworld_skybox() && r.native_shadow())
        });
        let shadows = self.settings.entity_shadows && self.pack.polytone_colors.entity_shadows();
        let shadow_sprite = (shadows && !pack_shadows)
            .then(|| self.gui.atlas.sprites.get("misc/shadow").copied())
            .flatten();
        if let Some(sprite) = shadow_sprite {
            let ambient = self.dimension_type().ambient_light;
            let darken = self.sky_darken();
            let a = alpha;
            let lerp = |p: Vec3, q: Vec3| {
                glam::DVec3::new(
                    p.x + (q.x - p.x) * a,
                    p.y + (q.y - p.y) * a,
                    p.z + (q.z - p.z) * a,
                )
            };
            // Each caster carries (interpolated placement pos, current distance pos, radius, strength).
            let mut casters: Vec<(glam::DVec3, glam::DVec3, f32, f32)> =
                sunrise_client::shadows::mob_casters(
                    self.mobs.iter().filter(|m| !self.hides_as_camera(m.id)),
                    a,
                )
                .map(|(pos, r, s)| (pos, pos, r, s))
                .collect();
            let item = sunrise_client::shadows::shadow_params("minecraft:item");
            casters.extend(
                self.dropped
                    .iter()
                    .filter(|i| !self.hides_as_camera(i.id))
                    .map(|i| {
                        let p = lerp(i.previous, i.position);
                        (p, p, item.0, item.1)
                    }),
            );
            let orb = sunrise_client::shadows::shadow_params("minecraft:experience_orb");
            casters.extend(
                self.orbs
                    .iter()
                    .filter(|o| !self.hides_as_camera(o.id))
                    .map(|o| {
                        let p = lerp(o.previous, o.position);
                        (p, p, orb.0, orb.1)
                    }),
            );
            // Text, block and item displays: same visibility gate, per-entity partial tick,
            // and current-position distance fade as `EntityRenderer.extractShadow`.
            let display_scale = self.view_scale();
            let display_now = std::time::Instant::now();
            casters.extend(self.text_displays.iter().filter_map(|(&id, d)| {
                let entity_partial = self.display_partial(id, a);
                if !self.display_visible(
                    id,
                    d,
                    camera,
                    entity_partial as f32,
                    display_scale,
                    display_now,
                ) {
                    return None;
                }
                let (pos, r, s) = d.shadow(entity_partial)?;
                Some((pos, d.position(), r, s))
            }));
            // Packs may move vertices or draw from another view.
            let shadow_view = self
                .view_frustum
                .as_ref()
                .filter(|_| self.pack_ids.is_none() && self.native_shaders.picked.is_empty());
            for (pos, dist_pos, radius, strength) in casters {
                if let Some(view) = shadow_view
                    && !sunrise_client::shadows::in_view_from(
                        view, camera, pos, dist_pos, radius, strength,
                    )
                {
                    continue;
                }
                sunrise_client::shadows::shadow_from(
                    &self.world,
                    self.res.facts(),
                    &|state| {
                        self.res
                            .collision()
                            .cached_shapes()
                            .cached_shape(state)
                            .ok()
                            .flatten()
                            .and_then(|shape| shape.bounds())
                    },
                    sprite,
                    camera,
                    pos,
                    dist_pos,
                    radius,
                    strength,
                    ambient,
                    darken,
                    &mut out,
                );
            }
        }
        laps.lap("eg_shadows");
        // Block entities (BlockEntityRenderers): chests, shulker boxes, banners, skulls...
        if let Some(r) = &self.renderer {
            // Block entities outside the view frustum are skipped (with a
            // block of margin for models larger than their block).
            let view = self.view_frustum;
            // Beams and structure outlines reach far outside their block
            // (BlockEntityRenderer.shouldRenderOffScreen): never culled.
            let off_screen = |state: u32| self.block_entity_reach(state).off_screen();
            // Sections past 64 blocks are kept for the End gateway's 256.
            let long_range = |state: u32| self.block_entity_reach(state) == Reach::EndGateway;
            let in_view = |p: &[i32; 3]| {
                view.is_none_or(|v| {
                    let o = (glam::DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]))
                        - camera)
                        .as_vec3();
                    v.intersects(o - glam::Vec3::ONE, o + glam::Vec3::splat(2.0))
                })
            };
            // One walk of the uploaded sections' block entities feeds both lists below
            // (`Renderer::block_entities_in_range` prunes by range alone): the camera's list,
            // and the sections the GPU cache warms.
            let mut camera_list: Vec<&([i32; 3], u32)> = Vec::new();
            let mut cache_candidates: Vec<&([i32; 3], u32)> = Vec::new();
            // A shader pack keeps the cache off (`render_with_cache_options_and_callbacks`),
            // so nothing warms it.
            let warm_cache = self.pack_ids.is_none();
            for entity in r.block_entities_in_range(
                self.settings.chunk_fade as f32,
                camera,
                view,
                self.effective_render_distance(),
                long_range,
                off_screen,
            ) {
                let (pos, state) = *entity.block;
                if !self.live_block_entity_render(pos, state) {
                    continue;
                }
                if off_screen(state) || entity.section_inside || in_view(&pos) {
                    camera_list.push(entity.block);
                }
                if warm_cache && entity.near {
                    cache_candidates.push(entity.block);
                }
            }
            // A shader pack's shadow pass draws the block entities of every section it draws,
            // in the view or not (`Renderer::shadow_pass_block_entities`, #8464): those the
            // view leaves out go in hidden from the camera.
            let shadow_only: Vec<&([i32; 3], u32)> =
                if self.pack_ids.is_some() && self.occlusion.shadow_casters.0 {
                    let listed: HashSet<[i32; 3]> = camera_list.iter().map(|b| b.0).collect();
                    r.shadow_pass_block_entities(camera)
                        .filter(|(pos, state)| {
                            !listed.contains(pos) && self.live_block_entity_render(*pos, *state)
                        })
                        .collect()
                } else {
                    Vec::new()
                };
            let shadow_only_at: HashSet<[i32; 3]> = shadow_only.iter().map(|b| b.0).collect();
            let dynamic_visible = |pos: [i32; 3], state: u32, lo: glam::DVec3, hi: glam::DVec3| {
                if off_screen(state) || shadow_only_at.contains(&pos) {
                    return true;
                }
                if let Some(view) = view
                    && !view.intersects(
                        (lo - camera - glam::DVec3::splat(0.5)).as_vec3(),
                        (hi - camera + glam::DVec3::splat(0.5)).as_vec3(),
                    )
                {
                    return false;
                }
                if self.settings.sunrise.entity_culling == EntityCullingMode::Off
                    || self.occlusion.shadow_casters.0
                    || self.block_entity_reach_entry(state).1
                    || hi.y - lo.y > 2.0
                {
                    return true;
                }
                // Culler::aabb_bounds adds 0.05 on each side. Keep its final
                // query just inside the conservative rendered-model envelope.
                let bounds = Aabb::new(
                    Vec3::new(lo.x + 0.06, lo.y + 0.06, lo.z + 0.06),
                    Vec3::new(hi.x - 0.06, hi.y - 0.06, hi.z - 0.06),
                );
                !self.block_entity_occluded(bounds)
            };
            self.with_block_entity_scene(camera, alpha as f32, &dynamic_visible, |scene| {
                let visible = camera_list.into_iter().chain(shadow_only.iter().copied());
                let block_entity_cache = CacheOptions {
                    shulker_boxes: self.settings.sunrise.cache_shulker_boxes,
                    signs: self.settings.sunrise.cache_signs,
                    decorated_pots: self.settings.sunrise.cache_decorated_pots,
                    bells: self.settings.sunrise.cache_bells,
                    chests: self.settings.sunrise.cache_chests,
                    banners: self.settings.sunrise.cache_banners,
                    copper_golem_statues: self.settings.sunrise.cache_copper_golem_statues,
                };
                // Entity occlusion is for ordinary entity meshes. Block entities
                // already have section and per-block frustum culling; routing them
                // through the per-vertex occlusion callback both repeats that work
                // and disables their per-section GPU cache. Shader packs still need
                // per-entity span marks, so they keep the callback path.
                if self.pack_ids.is_some() {
                    let section_revision = |pos| r.block_entity_section_revision(pos);
                    static_block_entity_stats = self
                        .res
                        .block_entity_render()
                        .render_with_cache_options_and_callbacks(
                            scene,
                            visible,
                            block_entity_cache,
                            r.block_entity_revision(),
                            Some(&section_revision),
                            &mut out,
                            |pos, state, start| {
                                self.mark_block_entity(start, state);
                                self.mark_motion(start, motion_block_entity(pos));
                            },
                            |pos, _, start, out| {
                                if shadow_only_at.contains(&pos) {
                                    self.hide_spans_from(start, out.len());
                                }
                            },
                        );
                } else {
                    (
                        static_block_entities,
                        visible_static_block_entity_sections,
                        static_block_entity_stats,
                    ) = self
                        .res
                        .block_entity_render()
                        .render_with_gpu_cache_options(
                            scene,
                            visible,
                            cache_candidates.into_iter(),
                            block_entity_cache,
                            r.block_entity_revision(),
                            |pos| r.block_entity_section_revision(pos),
                            &mut out,
                        );
                }
                // Mod block entities: their mods draw them (`native_block_entity_submit`),
                // culled by the eye-visible section filter (ordinary renderers) and the bounds
                // they give. A world without any (`has_mods`) has none to find among its other
                // unrendered block entities (furnaces, hoppers...).
                let radius = self.effective_render_distance();
                let untemplated = self.untemplated_mod_block_entities(camera);
                let mods = self
                    .world
                    .block_entities
                    .has_mods()
                    .then(|| {
                        r.visible_block_entities(
                            self.settings.chunk_fade as f32,
                            camera,
                            view,
                            radius,
                        )
                        .filter(|((_, state), _)| !self.res.block_entity_render().renders(*state))
                    })
                    .into_iter()
                    .flatten();
                let untemplated_eye = untemplated.iter().map(|block| {
                    (
                        block,
                        r.block_entity_section_in_eye_view(block.0, camera, view.as_ref(), radius),
                    )
                });
                self.mod_block_entity_geometry(
                    mods.chain(untemplated_eye),
                    camera,
                    alpha as f32,
                    &mut out,
                );
                self.mark_stage(out.len(), "DESTROY");
                for &(pos, stage) in &cracks {
                    let state = self.world.block(pos[0], pos[1], pos[2]);
                    self.res
                        .block_entity_render()
                        .render_breaking(scene, pos, state.0, stage, &mut out);
                }
            });
        }
        laps.lap("eg_block_entities");
        self.mark_motion(out.len(), 0);
        self.mark_entity(out.len(), "");
        self.moving_piston_geometry(camera, alpha, &mut out);
        // LevelRenderer.submitTransientBlocks.
        self.transient_block_geometry(camera, &mut out);
        self.mark_entity(out.len(), "minecraft:painting");
        self.painting_geometry(camera, alpha, &mut out);
        self.mark_motion(out.len(), 0);
        self.mark_entity(out.len(), "minecraft:item_frame");
        self.frame_geometry(camera, alpha, &mut out);
        self.mark_motion(out.len(), 0);
        self.cushion_geometry(camera, &mut out);
        // Beams: `gbuffers_beaconbeam`, not a block entity's program (5.4 has no stage for them).
        self.mark_stage(out.len(), "NONE");
        self.beacon_geometry(camera, alpha, &mut out);
        self.mark_entity(out.len(), "minecraft:fishing_bobber");
        self.fishing_geometry(camera, alpha, &mut out);
        self.mark_entity(out.len(), "minecraft:leash_knot");
        self.leash_geometry(camera, alpha, &mut out);
        self.mark_entity(out.len(), "minecraft:splash_potion");
        self.thrown_potion_geometry(camera, alpha, &mut out);
        laps.lap("eg_objects");
        // Breaking progress overlays (destroy_stage_0..9): ours and the other players'.
        self.mark_stage(out.len(), "DESTROY");
        for &(pos, stage) in &cracks {
            let o = (glam::DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]))
                - camera)
                .as_vec3();
            let state = self.world.block(pos[0], pos[1], pos[2]);
            if self.res.facts().get(state).render_shape
                == sunrise_client::blocks::RenderShape::Model
                && let Ok(models) = self
                    .pack
                    .render
                    .model_catalog
                    .collect(state, BlockPos::new(pos[0], pos[1], pos[2]))
            {
                let offset = self
                    .pack
                    .context
                    .tables
                    .offset(state, pos[0], pos[2])
                    .unwrap_or([0.0; 3]);
                let origin = o + glam::DVec3::from(offset).as_vec3();
                let translucent = models
                    .iter()
                    .any(|(_, model)| model.faces.iter().any(|f| f.translucent));
                for (_, model) in models {
                    sunrise_client::breaking::geometry(model, origin, translucent, stage, &mut out);
                }
            }
        }
        laps.lap("eg_cracks");
        self.finish_async_occlusion_culling();
        laps.lap("eg_culling_finish");
        (
            out,
            static_block_entities,
            visible_static_block_entity_sections,
            static_block_entity_stats,
        )
    }

    fn falling_setup(&self) -> FallingSetup<'_> {
        FallingSetup {
            // Packs can move vertices or draw another view; otherwise only depth
            // planes are used, which TAA's X/Y projection jitter cannot change.
            view: self
                .view_frustum
                .as_ref()
                .filter(|_| self.pack_ids.is_none() && self.native_shaders.picked.is_empty()),
            forward: self.camera_orientation() * -glam::Vec3::Z,
        }
    }

    /// `FallingBlockRenderer` for the falling blocks of `run`, drawn with their baked model,
    /// including non-cubic shapes.
    fn falling_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        setup: &FallingSetup<'_>,
        run: std::ops::Range<usize>,
        out: &mut Vec<Vertex>,
    ) {
        let FallingSetup {
            view: falling_view,
            forward: falling_forward,
        } = *setup;
        let tables = &self.pack.context.tables;
        for f in &self.falling[run] {
            if self.hides_as_camera(f.id) {
                continue;
            }
            let p = f.model_origin(alpha, |state, x, z| tables.offset(state, x, z)) - camera;
            let o = p.as_vec3();
            // Only rear-origin models pay for the envelope/depth check. The
            // full posed-camera test still decides rejection, including models
            // whose origin is behind but whose geometry reaches into view.
            if falling_forward.dot(o) < 0.0
                && falling_view.is_some_and(|view| {
                    self.pack
                        .render
                        .model_catalog
                        .render_bounds(f.state)
                        .is_some_and(|[lo, hi]| {
                            let lo = glam::Vec3::from_array(lo) + o;
                            let hi = glam::Vec3::from_array(hi) + o;
                            let magnitude = o
                                .abs()
                                .max_element()
                                .max(lo.abs().max_element())
                                .max(hi.abs().max_element());
                            let padding =
                                glam::Vec3::splat(1.0e-4 + magnitude * f32::EPSILON * 4.0);
                            !view.intersects_depth(lo - padding, hi + padding)
                        })
                })
            {
                continue;
            }
            // `FallingBlockEntity.blockPosition()`: the cell its feet are in.
            let (x, y, z) = (
                f.position.x.floor() as i32,
                f.position.y.floor() as i32,
                f.position.z.floor() as i32,
            );
            if !falling_block_visible(f.state, self.world.block(x, y, z)) {
                continue;
            }
            let Some((_, tint)) = self.falling_block_face_sprite(f.state) else {
                continue;
            };
            let light = self.world.light(x, y + 1, z);
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            let start = out.len();
            self.mark_motion(start, Self::MOTION_OBJECT | u64::from(f.id));
            self.falling_model_vertices(f.state, f.start_pos, o, packed, out);
            let tint = sunrise_client::mobs::render::abgr(tint);
            for vertex in &mut out[start..] {
                vertex.color = sunrise_client::hud_layout::multiply(vertex.color, tint);
            }
        }
    }
    /// The orb's sprite and the camera's axes, once a frame however many runs of orbs draw.
    pub(super) fn orb_frame(&self) -> OrbFrame {
        let sprite = self
            .gui
            .atlas
            .sprites
            .get("entity/experience/experience_orb")
            .copied();
        let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
        let forward = glam::Vec3::new(
            -yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        );
        let right = forward.cross(glam::Vec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        OrbFrame {
            sprite,
            forward,
            right,
            up,
        }
    }

    pub(super) fn orb_geometry<'a>(
        &self,
        camera: glam::DVec3,
        frame: &OrbFrame,
        orbs: impl Iterator<Item = (&'a sunrise_client::entities::ExperienceOrb, [f64; 3], f32)>,
        orb_view: Option<&sunrise_client::gfx::JoinedFrusta>,
        out: &mut Vec<Vertex>,
    ) {
        let OrbFrame {
            sprite,
            forward,
            right,
            up,
        } = *frame;
        for (o, at, partial) in orbs {
            let p = glam::DVec3::new(at[0], at[1] + 0.1, at[2]) - camera;
            let c = p.as_vec3();
            if orb_view
                .is_some_and(|view| forward.dot(c) < 0.0 && !entity_culling::orb_in_view(view, c))
            {
                continue;
            }
            if let Some(resolved) = native_entity_renderers::experience_orb_renderer() {
                let [width, height, eye_height] = o
                    .size_answer
                    .filter(|s| s.vanilla == sunrise_client::entities::ExperienceOrb::DIMENSIONS)
                    .and_then(|s| s.size)
                    .unwrap_or(sunrise_client::entities::ExperienceOrb::DIMENSIONS);
                let entity: mod_api::EntityRenderState =
                    mod_api::__private::built::EntityRenderState {
                        id: o.id,
                        partial,
                        age: o.age as f32 + partial,
                        yaw: o.rotation[0],
                        pitch: o.rotation[1],
                        position: at,
                        size: [width, height],
                        eye_height,
                        distance_to_camera_sq: (glam::DVec3::from_array(at) - camera)
                            .length_squared(),
                        living: None,
                        tnt_fuse: None,
                        orb_value: Some(o.value),
                        painting: None,
                        item_frame: None,
                        attachments: None,
                    }
                    .into();
                let light = self.world.light(
                    o.position.x.floor() as i32,
                    o.light_probe_y().floor() as i32,
                    o.position.z.floor() as i32,
                );
                let packed = o.render_light(i32::from(light));
                let block = [
                    o.position.x.floor() as i32,
                    o.light_probe_y().floor() as i32,
                    o.position.z.floor() as i32,
                ];
                if self.mod_entity_renderer_geometry(
                    &resolved,
                    &entity,
                    "minecraft:experience_orb",
                    glam::DVec3::from_array(at) - camera,
                    Some(glam::DVec3::from_array(at)),
                    block,
                    packed,
                    out,
                ) {
                    continue;
                }
            }
            let Some(r) = sprite else {
                continue;
            };
            let icon = o.icon();
            let (du, dv) = ((r.u1 - r.u0) / 4.0, (r.v1 - r.v0) / 4.0);
            let (u0, v0) = (r.u0 + (icon % 4) as f32 * du, r.v0 + (icon / 4) as f32 * dv);
            let t = (o.age as f32 + partial) / 2.0;
            let red = ((t.sin() + 1.0) * 0.5 * 255.0) as u32;
            let blue = (((t + 4.188_79).sin() + 1.0) * 0.1 * 255.0) as u32;
            let color = 0x8000_0000 | blue << 16 | 255 << 8 | red;
            // `ExperienceOrbRenderer.getBlockLightLevel` (the block light
            // plus 7, clamped) at the block the orb's eye is in.
            let light = self.world.light(
                o.position.x.floor() as i32,
                o.light_probe_y().floor() as i32,
                o.position.z.floor() as i32,
            );
            let packed = o.render_light(i32::from(light));
            // `ExperienceOrbRenderer.submit`: the quad runs from
            // (-0.5, -0.25) to (0.5, 0.75) and is scaled by 0.3, so the
            // sprite hangs a quarter of its height below the orb's centre.
            let s = sunrise_client::entities::ORB_QUAD_SCALE;
            for (k, uv) in sunrise_client::entities::ORB_QUAD.iter().zip([
                [u0, v0 + dv],
                [u0 + du, v0 + dv],
                [u0 + du, v0],
                [u0, v0],
            ]) {
                let v = c + right * (k[0] * s) + up * (k[1] * s);
                out.push(Vertex {
                    normal: [0; 4],
                    pos: v.to_array(),
                    uv,
                    // ExperienceOrbRenderer: `RenderTypes.entityTranslucent`; in HDR
                    // output the orbs shine (entity.wgsl fs_hdr_mark).
                    layer: sunrise_client::gfx::OIT_ENTITY
                        | u32::from(sunrise_client::mesher::VERY_BRIGHT) << 16,
                    color,
                    light: packed,

                    glint_uv: [0.0, 0.0],
                });
            }
        }
    }

    /// Extract every particle group after the frame's entity, screen and hand callbacks.
    /// Item model callbacks can clear during extraction too: roll back only this tail of
    /// geometry (including GPU jobs and outlines), never the frame's ordinary entities.
    fn client_particle_geometry(
        &mut self,
        camera: glam::DVec3,
        alpha: f64,
        fov_degrees: f32,
        providers: Option<(
            &sunrise_client::particles::ParticleCullView,
            &mut sunrise_client::particles::ParticleQuads,
        )>,
        out: &mut Vec<Vertex>,
    ) {
        self.apply_mod_particle_clear();
        // The packs' GPU particle records draw here too (a type without a shader the
        // renderer draws), and aren't in the native count.
        if self.client_particle_count() == 0 && self.particles.polytone_gpu_records() == 0 {
            return;
        }
        let first = out.len();
        let skin = self.skin_frame.borrow().mark();
        let outline_jobs = self.skin_frame.borrow().outline_jobs.len();
        let outline_vertices = self.skin_frame.borrow().outline_vertices;
        let spans = self.entity_spans.borrow().len();
        let motion = self.entity_motion.borrow().len();
        let glowing = self.glowing_vertices.borrow().len();
        self.mark_motion(out.len(), 0);
        // Particles (ParticleEngine.extract): camera-facing quads.
        self.mark_stage(out.len(), "PARTICLES");
        let world = &self.world;
        let particle_mode = self.settings.sunrise.particle_culling;
        let mut providers = providers;
        if particle_mode == sunrise_client::graphics::ParticleCullingMode::Off {
            self.particles.render_world(
                camera,
                self.particle_rotation(),
                alpha as f32,
                providers
                    .as_mut()
                    .map(|(view, quads)| (*view, &mut **quads)),
                None,
                &|b| world.light(b[0], b[1], b[2]),
                out,
            );
        } else {
            let frustum = self.view_frustum;
            let pixel_focal_length = self.renderer.as_ref().map_or(0.0, |renderer| {
                let tangent = (f64::from(fov_degrees).to_radians() * 0.5).tan();
                if tangent.is_finite() && tangent > 0.0 {
                    (f64::from(renderer.config.height.max(1)) / (2.0 * tangent)) as f32
                } else {
                    0.0
                }
            });
            {
                let mut particle_culling = self.particle_occlusion_context(camera);
                self.particles.render_world(
                    camera,
                    self.particle_rotation(),
                    alpha as f32,
                    providers
                        .as_mut()
                        .map(|(view, quads)| (*view, &mut **quads)),
                    Some(sunrise_client::particles::ParticleWorldCull {
                        pixel_focal_length,
                        frustum: frustum.as_ref(),
                        occluded: &mut |min, max| particle_culling.hidden(min, max),
                    }),
                    &|b| world.light(b[0], b[1], b[2]),
                    out,
                );
            }
            self.finish_async_particle_culling();
        }
        self.mark_entity(out.len(), "minecraft:item");
        self.pickup_item_geometry(camera, alpha, out);
        self.mark_motion(out.len(), 0);
        self.pickup_arrow_geometry(camera, alpha, out);
        self.mark_entity(out.len(), "minecraft:trident");
        self.pickup_trident_geometry(camera, alpha, out);
        self.mark_entity(out.len(), "minecraft:experience_orb");
        // ItemPickupParticle: the render state extracted at partial tick 1.
        let taken = self.pickups.live.iter().filter_map(|p| match &p.taken {
            pickups::Taken::Orb(o) => Some((&**o, pickups::position(p, alpha), 1.0)),
            _ => None,
        });
        let frame = self.orb_frame();
        self.orb_geometry(camera, &frame, taken, None, out);
        self.elder_apparition_geometry(alpha, out);
        if self.mod_clear_particles.get() {
            self.apply_mod_particle_clear();
            out.truncate(first);
            if let Some((_, quads)) = providers {
                quads.clear();
            }
            {
                let mut frame = self.skin_frame.borrow_mut();
                frame.truncate(skin);
                frame.outline_jobs.truncate(outline_jobs);
                frame.outline_vertices = outline_vertices;
            }
            self.entity_spans.borrow_mut().truncate(spans);
            self.entity_motion.borrow_mut().truncate(motion);
            self.glowing_vertices.borrow_mut().truncate(glowing);
            return;
        }
        self.mark_motion(out.len(), 0);
        self.mark_stage(out.len(), "NONE");
    }

    /// `ServerLevel.destroyBlockProgress`: breaker `breaker`'s crack on `pos`
    /// (0..=9, anything else takes it away), for every player within 64 blocks.
    pub(super) fn server_destroy_block_progress(
        &mut self,
        breaker: i32,
        pos: [i32; 3],
        progress: i32,
    ) {
        self.destroy_block_progress(breaker, pos, progress);
        if let Some(host) = self.lan.as_ref() {
            host.hub.broadcast_near(
                [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])],
                64.0,
                &sunrise_protocol::packets::play::Clientbound::BlockDestruction(
                    sunrise_protocol::packets::play::BlockDestruction {
                        id: breaker,
                        pos,
                        progress: if (0..10).contains(&progress) {
                            progress as u8
                        } else {
                            255
                        },
                    },
                ),
                None,
            );
        }
    }

    /// The mob hit along the view, its distance, and the hit point relative to its feet.
    fn mob_hit(&self) -> Option<(usize, f64, [f64; 3])> {
        if let Some(hit) = self.cached_entity_pick() {
            let (entity, distance, point) = hit?;
            let index = self.mobs.iter().position(|m| m.id == entity.id())?;
            let feet = self.mobs[index].position;
            return Some((
                index,
                distance,
                [point[0] - feet.x, point[1] - feet.y, point[2] - feet.z],
            ));
        }
        if let Some(hit) = self.tick_mob {
            return hit;
        }
        let pick_entity::CrosshairDecision::Entity(hit) = self.compute_crosshair_decision() else {
            return None;
        };
        let i = self.mob_index.find_ref(&self.mobs, hit.id)?;
        let point = Vec3::new(hit.point[0], hit.point[1], hit.point[2]);
        Some((
            i,
            hit.distance_sq.sqrt(),
            interact_location(point, self.mobs[i].position),
        ))
    }
    /// The brush's `ProjectileUtil.getHitResultOnViewVector` excludes only the source entity.
    pub(super) fn mob_hit_within(&self, reach: f64) -> Option<(usize, f64, [f64; 3])> {
        let source = self.pick_camera_id();
        let (eye, dir) = self.aim()?;
        let mut nearest = f64::INFINITY;
        let mut best: Option<(usize, f64, [f64; 3])> = None;
        for (i, m) in self.mobs.iter().enumerate() {
            if m.removed || Some(m.id) == source {
                continue;
            }
            if let Some(t) = ray_box(eye, dir, reach, m.bounds())
                && t != 0.0
                && t < nearest
            {
                nearest = t;
                let point = Vec3::new(
                    eye[0] + dir[0] * t,
                    eye[1] + dir[1] * t,
                    eye[2] + dir[2] * t,
                );
                best = Some((i, t, interact_location(point, m.position)));
            }
        }
        best
    }
    /// `Player.isSweepAttack` with the player's speed as it is, then
    /// `native_sweep_attack`, on either side: whether the hit on `target` sweeps.
    pub(super) fn sweep_decision(&mut self, scale: f32, crit: bool, target: &str) -> bool {
        let Some(p) = self.player.as_ref() else {
            return false;
        };
        let sword = self.held().is_some_and(|s| {
            let sword = self
                .res
                .items()
                .item_tags
                .get("minecraft:swords")
                .is_some_and(|tag| tag.contains(&*s.item));
            self.stack_can_perform(s, item_abilities::ability::SWORD_SWEEP, sword)
        });
        let vanilla = p.is_sweep_attack(scale, crit, sword);
        let (mut sweeping, mut canceled) = (vanilla, false);
        // `setCanceled` skips later mods and keeps the current `sweeping`.
        self.native_sweep_attack(target, vanilla, &mut sweeping, &mut canceled);
        sweeping
    }
    /// `Player.attack`'s `isSweepAttack`, decided before the hit as vanilla does
    /// (`crit`: the hit on a living target is critical): where the sweep plays,
    /// none without one. Reads the charge, so before the hit resets it.
    pub(super) fn sweep_before_hit(&mut self, target: &str, crit: bool) -> Option<[f64; 3]> {
        let scale = self.attack_strength_scale();
        self.sweep_attack_position(scale, crit, target)
    }
    /// `Player.attack`'s sounds when a vehicle takes the hit (it always does, and
    /// can't be hit critically): knockback when sprinting at full strength, then
    /// strong or weak, or the `sweep` decided before the hit around the vehicle's
    /// box `area`. Before the hit resets the charge (`Player.onAttack`).
    pub(super) fn vehicle_attack_sounds(
        &mut self,
        area: Aabb,
        sweep: Option<[f64; 3]>,
        magic: Option<i32>,
    ) {
        self.attack_effects(area, sweep, None, false, mace::Spared::Nothing, magic);
    }
    /// A mob explosion (`ServerLevel.explode` with `ExplosionInteraction.MOB`:
    /// blocks are kept without mobGriefing, else the mob_explosion_drop_decay
    /// rule picks decay). Mods are asked through `can_entity_grief` for
    /// `source`, the exploding entity (the rule alone without one).
    fn explode_with(
        &mut self,
        source: Option<u32>,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        blast: explosion::ExplosionSource,
    ) -> Result<()> {
        let interaction = if !self.can_entity_grief(source) {
            explosion::BlockInteraction::Keep
        } else {
            self.destroy_type(sunrise_client::gamerules::vanilla::MOB_EXPLOSION_DROP_DECAY)
        };
        self.server_explode_from(center, radius, interaction, fire, blast)
    }
    /// Copies of a save batch carrying its live block entities and save data.
    fn with_entities<'a>(
        &self,
        chunks: impl IntoIterator<Item = &'a Arc<sunrise_client::chunk::Chunk>>,
    ) -> Vec<Arc<sunrise_client::chunk::Chunk>> {
        sunrise_client::save::chunks_with_block_entities(
            chunks,
            &self.world.block_entities,
            &self.world.block_names,
        )
        .into_iter()
        .map(|mut c| {
            self.attach_save_data(&mut c);
            if mod_api::__private::has_part_hook(mod_api::HookId::BlockEntitySaveAdditional) {
                let edits: std::collections::BTreeMap<_, _> = c
                    .block_entities
                    .iter()
                    .filter_map(|(pos, entity)| {
                        let BlockEntity::Mod(entity) = entity else {
                            return None;
                        };
                        if !matches!(
                            self.world.block_entities.get(pos),
                            Some(BlockEntity::Mod(_))
                        ) {
                            return None;
                        }
                        let registry = self.pack.assets.registry;
                        let block = registry
                            .state(c.block(pos.0 & 15, pos.1, pos.2 & 15))
                            .map_or("", |state| state.name.as_str());
                        self.capture_block_entity_payload(*pos, block, entity, self.res.items())
                            .map(|edited| (*pos, edited))
                    })
                    .collect();
                if !edits.is_empty() {
                    Arc::make_mut(c.saved.get_or_insert_with(Default::default))
                        .block_entity_save_edits = Some(Arc::new(edits));
                }
            }
            Arc::new(c)
        })
        .collect()
    }
    fn finish_open_world(
        &mut self,
        mut prepared: world_loading::PreparedWorld,
    ) -> Result<functions::PackStack> {
        self.reset_tutorial();
        // Another world: its generator, not the last one's Density line.
        self.debug_density.borrow_mut().clear();
        self.server_loot = Default::default();
        self.world_root.clone_from(&prepared.save_root);
        // The root moves before the prepared level is installed. Until then a
        // final save must not run: the lock below only marks tag tables.
        self.world_adopted = false;
        self.owner_migration = prepared.owner_migration;
        sunrise_client::crash::set_detail(
            "World",
            format!("{} ({})", prepared.level.name, prepared.save_root.display()),
        );
        self.score = Default::default();
        reset_world_progress(&mut self.adv, &mut self.stats);
        self.reset_creative_contents();
        let features = prepared.level.enabled_features.clone();
        let (stack, terrain) = self
            .open_world_datapacks(&features, Some(&mut prepared))
            .map_err(|error| error.diagnostic())?;
        let world_loading::PreparedTerrain {
            configured_levels,
            world,
            streaming,
            save,
            ..
        } = terrain.expect("opening a world prepares its terrain");
        let world_loading::PreparedWorld {
            save_root,
            level,
            session_lock,
            rand_sequences,
            ..
        } = prepared;
        let dim = Dim::from_id(&level.dimension);
        let root = self.options.resources.clone();
        self.damage_tag_generation = self.damage_tag_generation.wrapping_add(1);
        // A code-mod switch can have rebuilt the root since `Game::new`.
        self.structure_spawns = sunrise_client::mobs::spawn_selection::StructureSpawns::load(&root);
        // Native tag reads use the lock to tell that the tag tables are ready.
        self.session_lock = Some(session_lock);
        self.resolve_parts()?;
        self.refresh_recipe_property_context();
        // Warm this world's pack-aware loot data before gameplay can unpack
        // a container's pending table on its first inventory access.
        self.load_server_loot();
        // MinecraftServer's constructor reads CustomBossEvents before any
        // server event.
        self.bosses = Default::default();
        self.load_bossbars();
        // ServerLevel's constructor reads its Raids saved data before any
        // server event; their randoms are seeded on the first raid tick.
        if self.remote.is_none() {
            self.read_raids();
        }
        self.announce_data_bound(mod_api::TagsUpdateCause::ServerDataLoad { reload: false });
        self.announce_data_maps_updated(mod_api::DataMapsUpdateCause::ServerReload);
        self.sync_recipe_content();
        self.restore_player_state = true;
        self.rand_sequences = rand_sequences;
        // ServerLevel.getWorldBorder: the level in place reads its own saved
        // border when it is made, before any command (the parked levels read
        // theirs in `open_level`).
        self.border = self.load_border(dim);
        self.set_mode_id(level.mode());
        self.previous_mode = level.previous_game_mode.map(game_mode::mode_from_id);
        let mut inventory = self.load_saved_inventory(&level);
        if self.creative && level.inventory.is_empty() && level.position.is_none() {
            for (i, name) in [
                "minecraft:grass_block",
                "minecraft:stone",
                "minecraft:oak_planks",
                "minecraft:glass",
                "minecraft:torch",
                "minecraft:oak_log",
                "minecraft:chest",
                "minecraft:furnace",
                "minecraft:diamond_pickaxe",
            ]
            .iter()
            .enumerate()
            {
                inventory[i] = Some(ItemStack {
                    item: Arc::from(*name),
                    count: 1,
                    damage: 0,
                    extra: None,
                });
            }
        }
        let (yaw, pitch) = self
            .options
            .look
            .unwrap_or((level.rotation[0], level.rotation[1]));
        self.yaw = yaw;
        self.pitch = pitch;
        self.selected = level.selected_slot.clamp(0, 8) as usize;
        self.bind_chunk_disk_worker(&save, dim);
        self.streaming = streaming;
        self.world = world;
        self.chunk_spawn_enemies = true;
        self.inventory = inventory;
        self.save = self.with_profession_save_snapshot(save);
        // MinecraftServer.prepareLevels restores the saved tickets after the
        // built-in levels have been published.
        self.tickets = Default::default();
        self.startup_ticket_restore_pending = self.options.execution_role.is_logical_server();
        // The ride the player's file names, once the vehicle is in the world.
        self.pending_vehicle = level.root_vehicle;
        self.level = level;
        self.check_mod_mismatch();
        self.world_adopted = true;
        if self.options.execution_role.is_logical_server() {
            self.options
                .save_failure_diagnostics
                .register(self.save.save_failure_reporter());
        }
        // ServerRecipeBook.loadUntrusted: recipes the manager does not have are dropped.
        let index = &world_stations!(self).recipe_index;
        let book = &mut self.level.recipe_book;
        book.recipes.retain(|r| index.contains(r));
        book.to_be_displayed.retain(|r| index.contains(r));
        self.enter_opened_dim(dim);
        self.audio.set_world(dim.id());
        // LevelStorageAccess.getIconFile: this save's icon.png is what the
        // Select World list shows.
        self.world_icon = Some(WorldIcon {
            path: save_root.join("icon.png"),
            last_attempt: None,
        });
        self.world_flat = sunrise_client::save::WorldGen::read(&save_root)
            .is_some_and(|w| w.preset == sunrise_client::save::WorldPreset::Flat);
        self.world_debug = sunrise_client::save::WorldGen::read(&save_root)
            .is_some_and(|w| w.preset == sunrise_client::save::WorldPreset::Debug);
        self.block_ticks = Ticks::new(self.level.seed as i32);
        self.tick_chunks.clear();
        self.clear_entities();
        self.player = None;
        self.sleeping = None;
        self.sleep_timer = 0;
        self.player_beds.clear();
        self.spawn = None;
        self.respawn_config = self
            .level
            .respawn_elsewhere
            .as_ref()
            .and_then(respawn_anchor::RespawnConfig::from_saved);
        self.spawn_origin = None;
        self.spawn_hook_fired = false;
        self.spawn_hook_cancelled = false;
        self.spawn_spiral = Default::default();
        self.player_spawn = None;
        self.creatures.reset_trader();
        self.creatures.cat = Default::default();
        self.creatures.custom_spawners.clear();
        self.portals = Default::default();
        self.experience = Experience {
            level: self.level.xp_level,
            progress: self.level.xp_progress,
            total: self.level.xp_total,
        };
        self.player_score = self.level.player_score;
        self.restore_effects();
        self.mining = None;
        self.container = None;
        let kinds = std::mem::take(&mut self.be_anims.kinds);
        self.be_anims = sunrise_client::block_entity_anim::BlockEntityAnims {
            kinds,
            ..Default::default()
        };
        self.carried = None;
        self.using = None;
        self.set_screen(Screen::Playing);
        self.hide_title();
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
        self.apply_settings();
        let facts = self.dimension_type_of(dim);
        self.streaming.sky_light = facts.height.has_skylight;
        self.streaming.nether_shading =
            facts.cardinal_light == sunrise_world::dimension_type::CardinalLight::Nether;
        self.load_world_state()?;
        // MinecraftServer.createLevels loads scoreboard, command storage, and
        // stopwatches before firing the Overworld's LevelEvent.Load.
        if self.remote.is_none() {
            self.load_scoreboard();
        }
        self.open_configured_levels(&configured_levels)?;
        Ok(stack)
    }
    fn finish_close_world(&mut self) -> Result<()> {
        self.ambience.clear(&mut self.audio);
        // updateLevelInEngines(null): every sound stops; the music manager
        // lets go of its track.
        self.music.stop_playing(None, &mut self.audio);
        self.audio.leave_level();
        if self.remote.is_some() {
            self.world_icon = None;
            return self.leave_server(None);
        }
        // The save screen has finished every write before releasing storage.
        self.ui.dispatcher = Default::default();
        self.reset_creative_contents();
        self.server_loot.set(None);
        self.server_items = None;
        self.session_lock = None;
        self.clear_world_parts();
        self.world_adopted = false;
        self.startup_ticket_restore_pending = false;
        self.world_icon = None;
        sunrise_client::crash::clear_detail("World");
        self.debug_charts.reset();
        self.boss_overlay.reset();
        // GameRenderer.resetData (Minecraft.clearClientLevel).
        self.item_activation.reset();
        self.kept_block_entities.get_mut().clear();
        self.mod_view_distances.take();
        self.bosses = Default::default();
        self.streaming.paused = true;
        self.world = Game::world_for_mods(self.res.facts().air, 0);
        self.player = None;
        // `LocalPlayer`'s constructor: `KeyMapping.resetToggleKeys`, so a sneak or sprint toggled
        // in this world is not still on in the next one.
        self.reset_toggle_keys();
        self.sleeping = None;
        self.sleep_timer = 0;
        self.player_beds.clear();
        self.clear_entities();
        self.set_screen(Screen::Playing);
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
        // Clear sync data (registries, dialog tags) so dialogs don't leak
        // between worlds.
        lan::sync::forget_sync_data();
        lan::sync::forget_server_sources();
        let mut title = self
            .new_title(std::env::current_dir()?.join("saves"))
            .with_user(|| self.options.player_name());
        title.last_server = self.settings.file.string("lastServer");
        title.skip_multiplayer_warning = self.settings.file.bool("skipMultiplayerWarning");
        self.show_title(title);
        self.capture(false);
        Ok(())
    }
    /// Let go of every entity of the level (saved first): the next world
    /// opened must not inherit them, nor save them as its own.
    pub(super) fn clear_entities(&mut self) {
        self.clear_pending_mob_actions();
        self.mobs.clear();
        self.dropped.clear();
        self.orbs.clear();
        self.creatures.projectiles.clear();
        self.falling.clear();
        self.transient_blocks.clear();
        self.tnts.clear();
        self.boats.clear();
        self.carts.clear();
        self.armor_stands.clear();
        self.cushions.clear();
        self.item_frames.clear();
        self.paintings.clear();
        self.others.clear();
        self.dragon.discard();
        self.crystals.clear();
        self.breath_clouds.clear();
        self.tridents.clear();
        self.rockets.clear();
        self.bolts.clear();
        self.clear_client_particles();
        self.riding = None;
        self.persist = Default::default();
        self.missing_professions.clear();
        self.entity_attachments.clear();
        self.clear_remote_objects();
        self.clear_remote_projectiles();
        self.levels.clear();
        self.entity_vertices.clear();
        self.see_through_vertices.clear();
        self.glowing_vertices.get_mut().clear();
        self.shadow_only_vertices.get_mut().clear();
        self.entity_spans.borrow_mut().clear();
        self.entity_motion.borrow_mut().clear();
        self.entity_motion_posed.borrow_mut().clear();
        self.skin_models
            .borrow_mut()
            .restart(&mut self.skin_frame.borrow_mut());
        self.skin_frame.borrow_mut().clear_draws();
    }
    /// Voxel traversal that also stops at fluid source blocks (bucket pick).
    fn pick_fluid(&self) -> Option<Hit> {
        let (origin, dir) = self.aim()?;
        let mut t = 0.0;
        let mut last = origin.map(|v| v.floor() as i32);
        let mut first = true;
        while t <= self.reach() {
            let p = [
                origin[0] + dir[0] * t,
                origin[1] + dir[1] * t,
                origin[2] + dir[2] * t,
            ];
            let b = p.map(|v| v.floor() as i32);
            // `BlockGetter.traverseBlocks` visits the block the ray starts in
            // first: the eyes' own water, under water (#3787).
            if first || b != last {
                let state = self.world.block(b[0], b[1], b[2]);
                let f = self.res.facts().get(state);
                let face = [last[0] - b[0], last[1] - b[1], last[2] - b[2]];
                if f.fluid != Fluid::None
                    && f.fluid_level == 8
                    && !f.fluid_falling
                    && !f.waterlogged
                    && f.render_shape != sunrise_client::blocks::RenderShape::Model
                {
                    if !first {
                        return Some(Hit {
                            pos: b,
                            face,
                            point: p,
                            inside: false,
                        });
                    }
                    let above = self.world.block(b[0], b[1] + 1, b[2]);
                    let same_above = self.res.facts().get(above).fluid == f.fluid;
                    if let Some((face, point)) =
                        clip_start_source(origin, dir, self.reach(), b, same_above)
                    {
                        return Some(Hit {
                            pos: b,
                            face,
                            point,
                            // A start in the shape hits at the eyes.
                            inside: point == origin,
                        });
                    }
                } else if !first && !self.res.facts().is_air(state) && f.fluid == Fluid::None {
                    // `ClipContext.Block.OUTLINE`: the ray stops at the block's
                    // outline, not its cell; it passes through the empty part
                    // of a torch, a fence or a flower to the water behind.
                    if let Some(hit) =
                        outline_stop(origin, dir, self.reach(), b, face, || self.outline_boxes(b))
                    {
                        return Some(hit);
                    }
                }
                last = b;
                first = false;
            }
            t += 0.05;
        }
        None
    }
    /// The cauldron at `pos`, and whether water is above it
    /// (`CauldronInteractions.isUnderWater`).
    pub(super) fn cauldron_at(
        &self,
        pos: [i32; 3],
    ) -> Option<(sunrise_world::chunk::BlockState, bool)> {
        let registry = self.pack.assets.registry;
        let id = self.world.block(pos[0], pos[1], pos[2]);
        let state = registry
            .state(id)
            .filter(|s| cauldron_interactions::snapshot().contains_block(&s.name))
            .cloned()?;
        let above = self.world.block(pos[0], pos[1] + 1, pos[2]);
        let under_water = cauldrons::is_under_water(self.res.facts(), above);
        Some((state, under_water))
    }

    /// What `BlockTransformer.transformBlock` would do with a held axe, hoe
    /// or shovel on the block at `pos`, without doing it: the new state, its
    /// sound, loot table, the durability it costs, its particle and its
    /// transform type.
    pub(super) fn block_transform(
        &self,
        stack: &ItemStack,
        hit: &Hit,
    ) -> Option<(
        BlockStateId,
        String,
        Option<String>,
        u32,
        String,
        TransformKind,
    )> {
        let blocking = self
            .offhand
            .as_ref()
            .is_some_and(|s| self.blocks_attacks(s).is_some())
            && !self.held_mapping(Mapping::Sneak);
        self.block_transform_in(
            stack,
            hit,
            TransformContext {
                world: &self.world,
                player: self.player.is_some().then(|| self.own_entity_id() as u32),
                hand: self.hand,
                blocking,
                simulated: false,
            },
        )
    }

    fn block_transform_in(
        &self,
        stack: &ItemStack,
        hit: &Hit,
        context: TransformContext<'_>,
    ) -> Option<(
        BlockStateId,
        String,
        Option<String>,
        u32,
        String,
        TransformKind,
    )> {
        use sunrise_client::block_transform::Around;
        let TransformContext {
            world,
            player,
            hand,
            blocking,
            simulated,
        } = context;
        let Hit { pos, face, .. } = *hit;
        struct Column<'w> {
            world: &'w World,
            registry: &'w sunrise_world::registry::BlockRegistry,
            pos: [i32; 3],
        }
        impl Around for Column<'_> {
            fn state(&self, o: [i32; 3]) -> sunrise_world::chunk::BlockState {
                let id =
                    self.world
                        .block(self.pos[0] + o[0], self.pos[1] + o[1], self.pos[2] + o[2]);
                self.registry.state(id).cloned().unwrap_or_else(|| {
                    sunrise_world::chunk::BlockState {
                        name: "minecraft:air".into(),
                        properties: Default::default(),
                    }
                })
            }
        }
        let id = self
            .res
            .items()
            .get(&stack.item)
            .and_then(|i| i.block_transformer.clone())?;
        // playerHasBlockingItemUseIntent: a shield in the off hand is raised
        // instead, unless sneaking.
        if blocking {
            return None;
        }
        let face_name = match face {
            [0, -1, 0] => "down",
            [0, 1, 0] => "up",
            [0, 0, -1] => "north",
            [0, 0, 1] => "south",
            [-1, 0, 0] => "west",
            _ => "east",
        };
        let registry = self.pack.assets.registry;
        let block = world.block(pos[0], pos[1], pos[2]);
        let around = Column {
            world,
            registry,
            pos,
        };
        let transformers = self.res.transformers();
        let vanilla = transformers
            .apply(self.items(), registry, &id, face_name, &around)
            .and_then(|(_, t, state)| Some((registry.resolve(&state).ok()?, t)));
        let result = |new, t: &sunrise_client::block_transform::Transform| {
            (
                new,
                t.sound.clone(),
                t.loot.clone(),
                t.damage,
                t.particle.clone(),
                TransformKind {
                    transform_type: t.transform_type.clone(),
                    drop_strategy: t.drop_strategy.clone(),
                },
            )
        };
        let mapped = self.axe_mapped(&id, block);
        if mod_api::__private::mods().is_empty() {
            if let Some((new, ability)) = mapped {
                return Some(Self::tool_ability_result(new, ability));
            }
            let (new, t) = vanilla?;
            return Some(result(new, t));
        }
        use mod_api::ToolAbility::*;
        let actions: &[_] = match id.as_str() {
            "minecraft:axe" => &[AxeStrip, AxeScrape, AxeWaxOff],
            "minecraft:shovel" => &[ShovelFlatten],
            "minecraft:hoe" => &[HoeTill],
            _ => return vanilla.map(|(new, t)| result(new, t)),
        };
        for &ability in actions {
            let mut state = match mapped {
                Some((new, from)) => (from == ability).then_some(new),
                None => vanilla
                    .as_ref()
                    .and_then(|(state, t)| vanilla_tool_state(&id, &t.particle, ability, *state)),
            };
            let matches_vanilla = state.is_some() && mapped.is_none();
            let mut cancel = false;
            let tool = mod_api::ToolUse::from(mod_api::__private::built::ToolUse {
                player,
                hand,
                pos,
                face,
                click: hit.point,
                ability,
                simulated,
            });
            self.mod_tool_modification(stack, block, &tool, &mut state, &mut cancel);
            if cancel {
                return None;
            }
            if let Some(new) = state {
                if matches_vanilla {
                    let (_, t) = vanilla?;
                    return Some(result(new, t));
                }
                return Some(Self::tool_ability_result(new, ability));
            }
        }
        None
    }
    /// `BlockTransformer.transformBlock` for a held axe, hoe or shovel.
    fn use_block_transformer(&mut self, stack: &ItemStack, hit: &Hit) -> Result<bool> {
        let pos = hit.pos;
        let face = hit.face;
        let Some((new, sound, loot, damage, particle, kind)) = self.block_transform(stack, hit)
        else {
            return Ok(false);
        };
        let transform_type = kind.transform_type;
        // BlockTransformer.transformBlock: ITEM_USED_ON_BLOCK before the swap.
        self.trigger_block("minecraft:item_used_on_block", pos, stack);
        // Block.dropFromBlockInteractLootTable of the transform's own loot table, popped by its
        // drop strategy, before the block changes.
        if let Some(table) = loot {
            let state = self.world.block(pos[0], pos[1], pos[2]);
            let actor = self.interacting_loot_entity();
            for drop in self.interact_loot_at(&table, pos, state, Some(actor), Some(stack)) {
                if kind.drop_strategy == "clicked_face" {
                    self.pop_resource_from_face(pos, face, drop);
                } else {
                    self.pop_resource(pos, drop);
                }
            }
        }
        let old = self.world.block(pos[0], pos[1], pos[2]);
        self.level_set(pos, new)?;
        // BlockTransformer.TransformParticle.send.
        self.transform_particle(&particle, pos);
        // `Level.playSound(player, ...)`: the actor's client already played
        // the strip/scrape/till sound in `predict_item_on_block` (#4681).
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        if self.lan_acting.is_some() {
            self.play_audio_unshared(&sound, Some(at), 1.0, 1.0);
            self.guest_player_sounds.push((sound.clone(), at, 1.0, 1.0));
        } else {
            self.play_audio(&sound, Some(at), 1.0, 1.0);
        }
        // ItemStack.useOn: a use that acted counts as one of the item.
        self.award_stat("used", &stack.item, 1);
        self.damage_held(damage);
        self.post_block_game_event("minecraft:block_change", pos, self.sculk_actor(), Some(new));
        if transform_type == "copper_chest" {
            self.transform_copper_chest_neighbour(pos, old, &particle);
        }
        Ok(true)
    }
    /// `HoneycombItem.useOn`'s `Level.playSound(player, ...)`: the actor's client
    /// already played it in `predict_item_on_block`.
    fn wax_on_sound(&mut self, at: [f64; 3]) {
        let sound = "minecraft:item.honeycomb.wax_on";
        if self.lan_acting.is_some() {
            self.play_audio_unshared(sound, Some(at), 1.0, 1.0);
            self.guest_block_sounds
                .push((sound.to_owned(), at, 1.0, 1.0));
        } else {
            self.play_audio(sound, Some(at), 1.0, 1.0);
        }
    }
    /// `BlockTransformer.transformBlock`'s `COPPER_CHEST` extra: a double
    /// chest's other half hears a block change and gets the same particle.
    fn transform_copper_chest_neighbour(
        &mut self,
        pos: [i32; 3],
        old: BlockStateId,
        particle: &str,
    ) {
        let old = self.pack.assets.registry.state(old).cloned();
        let Some(old) = old else { return };
        if !old.name.ends_with("copper_chest") {
            return;
        }
        let kind = old
            .properties
            .get("type")
            .map(String::as_str)
            .unwrap_or("single");
        let Some(facing) = old.properties.get("facing").map(String::as_str) else {
            return;
        };
        // `ChestBlock.getConnectedDirection`: LEFT is facing.getClockWise().
        let (dx, dz) = match (kind, facing) {
            ("left", "north") => (1, 0),
            ("left", "south") => (-1, 0),
            ("left", "west") => (0, -1),
            ("left", "east") => (0, 1),
            ("right", "north") => (-1, 0),
            ("right", "south") => (1, 0),
            ("right", "west") => (0, 1),
            ("right", "east") => (0, -1),
            _ => return,
        };
        let neighbour = [pos[0] + dx, pos[1], pos[2] + dz];
        let state = self.world.block(neighbour[0], neighbour[1], neighbour[2]);
        self.post_block_game_event(
            "minecraft:block_change",
            neighbour,
            self.sculk_actor(),
            Some(state),
        );
        self.transform_particle(particle, neighbour);
    }
    fn set_state_named(&mut self, pos: [i32; 3], name: &str, keep: &[&str]) -> Result<bool> {
        let registry = &self.pack.assets.registry;
        let old = self.world.block(pos[0], pos[1], pos[2]);
        let Some(default) = registry.default_state(name) else {
            return Ok(false);
        };
        let mut state = registry.state(default).cloned().ok_or("state")?;
        if let Some(previous) = registry.state(old) {
            for k in keep {
                if let Some(v) = previous.properties.get(*k) {
                    state.properties.insert((*k).to_owned(), v.clone());
                }
            }
        }
        let id = registry.resolve(&state).unwrap_or(default);
        self.level_set(pos, id)?;
        Ok(true)
    }
    /// Block tool use: distinguish success, failure and fall-through for hand feedback.
    fn use_tool(
        &mut self,
        stack: &ItemStack,
        allow_block_actions: bool,
    ) -> Result<sunrise_client::first_person::UseResult> {
        use sunrise_client::first_person::UseResult;
        let item = stack.item.trim_start_matches("minecraft:").to_owned();
        let at = |p: [i32; 3]| {
            [
                f64::from(p[0]) + 0.5,
                f64::from(p[1]) + 0.5,
                f64::from(p[2]) + 0.5,
            ]
        };
        // BucketItem has no useOn: a guest's UseItemOn leaves it to UseItem.
        let bucket = sunrise_client::items::bucket_contents(&stack.item);
        if matches!(self.guest_use, Some(GuestUse::OnBlock { .. }))
            && (item == "bucket" || bucket.is_some())
        {
            return Ok(UseResult::Pass);
        }
        if item == "bucket" {
            let Some(hit) = self.bucket_hit() else {
                return Ok(UseResult::Pass);
            };
            // `BucketItem.use`: `level.mayInteract` and `mayUseItemAt`.
            if self.bucket_use_refused(hit.pos) {
                return Ok(UseResult::Fail);
            }
            let mut canceled = false;
            self.native_fill_bucket(stack, hit.pos, &mut canceled);
            if canceled {
                return Ok(UseResult::Fail);
            }
            if self.is_powder_snow(hit.pos) {
                self.pick_up_powder_snow(hit.pos)?;
                return Ok(UseResult::Success { swing: true });
            }
            // `BucketPickup.pickupBlock`: a fluid source, a waterlogged block
            // or powder snow. A block that gives nothing fails the click.
            let pos = BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
            let creative = self.creative;
            let Some(filled) =
                self.with_level(|b, level, ticks| b.bucket_pickup(level, ticks, pos, creative))
            else {
                return Ok(UseResult::Fail);
            };
            self.process_level_events()?;
            // `player.awardStat(Stats.ITEM_USED.get(this))`,
            // `getPickupSound` and `level.gameEvent(player, FLUID_PICKUP)`.
            self.award_stat("used", "minecraft:bucket", 1);
            let sound = if filled == "minecraft:lava_bucket" {
                "minecraft:item.bucket.fill_lava"
            } else {
                "minecraft:item.bucket.fill"
            };
            self.play_audio(sound, Some(at(hit.pos)), 1.0, 1.0);
            self.post_block_game_event(
                "minecraft:fluid_pickup",
                hit.pos,
                Some(sunrise_client::mobs::Who::Player),
                None,
            );
            let filled = ItemStack::new(filled, 1);
            self.filled_result(filled.clone());
            self.trigger_item("minecraft:filled_bucket", &filled);
            return Ok(UseResult::Success { swing: true });
        }
        // BucketItem.use has its own ray, even when the crosshair hits an entity.
        if !allow_block_actions && bucket.is_none() {
            return Ok(UseResult::Pass);
        }
        let hit = if bucket.is_some() {
            self.item_pick()
        } else {
            self.pick()
        };
        let Some(hit) = hit else {
            return Ok(UseResult::Pass);
        };
        let clicked = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        if let Some((content, sound, mob)) = bucket {
            if self.bucket_use_refused(hit.pos) {
                return Ok(UseResult::Fail);
            }
            let target = self.bucket_target(content, &hit);
            // `mayUseItemAt(blockPos2)`: a block that refuses the fluid passes
            // it to the block beside, so a mod hears that block too (powder
            // snow places like a block and never goes there).
            let beside_too = content != BucketContent::Powder && hit.beside() != target;
            if self.bucket_empty_cancelled(stack, target)
                || beside_too && self.bucket_empty_cancelled(stack, hit.beside())
            {
                return Ok(UseResult::Fail);
            }
            if let Some(result) = self.player_empty_contents(stack, target, &hit) {
                return Ok(result);
            }
            let pos = BlockPos::new(target[0], target[1], target[2]);
            let [x, y, z] = hit.beside();
            let beside = Some(BlockPos::new(x, y, z));
            // `Player.isShiftKeyDown`: a block that refuses the fluid (any
            // non-air block under Shift) passes it to the block beside, where
            // Shift no longer counts.
            let sneaking = self.held_mapping(Mapping::Sneak);
            let emptied = self.empty_player_bucket(pos, beside, content, sound, sneaking);
            self.process_level_events()?;
            let Some(emptied) = emptied else {
                return Ok(UseResult::Fail);
            };
            if self.dimension_type().water_evaporates && content.is_water() {
                self.evaporation_smoke([emptied.x, emptied.y, emptied.z]);
            }
            // `BucketItem.emptyContents` uses `playSound(player, …)`: the
            // actor's client already played the empty / evaporate sound from
            // its DryLevel prediction (#4684).
            self.keep_guest_bucket_sounds(sound);
            if let Some(kind) = mob {
                if let Some(id) = self.spawn_mob_at(kind, (target[0], target[1], target[2]), true) {
                    self.mob_placed(id, stack)?;
                }
                self.post_block_game_event(
                    "minecraft:entity_place",
                    target,
                    self.sculk_actor(),
                    None,
                );
            }
            self.trigger_block("minecraft:placed_block", target, stack);
            self.bucket_emptied(&stack.item);
            return Ok(UseResult::Success { swing: true });
        }
        if self.may_use_on(hit.pos) && self.use_block_transformer(stack, &hit)? {
            return Ok(UseResult::Success { swing: true });
        }
        if self.may_use_on(hit.pos) && item == "honeycomb" {
            let registry = self.pack.assets.registry;
            let old = registry.state(clicked);
            if let Some(state) = self.honeycomb_waxed(clicked) {
                let other = old.and_then(|old| storage::chest_connected_pos(old, hit.pos));
                // HoneycombItem.useOn: ITEM_USED_ON_BLOCK before the swap.
                self.trigger_block("minecraft:item_used_on_block", hit.pos, stack);
                self.level_set(hit.pos, state)?;
                let source = self
                    .lan_acting_uuid
                    .map(sunrise_world::level::LevelEventPlayer);
                self.level_event_from(source, 3003, hit.pos, 0);
                self.wax_on_sound(at(hit.pos));
                // HoneycombItem.useOn's game event, and ItemStack.useOn's used statistic.
                self.post_block_game_event(
                    "minecraft:block_change",
                    hit.pos,
                    self.sculk_actor(),
                    Some(state),
                );
                // A double chest's other half: its game event and particles, and
                // a second sound from the clicked block.
                if let Some(other) = other {
                    let now = self.world.block(other[0], other[1], other[2]);
                    self.post_block_game_event(
                        "minecraft:block_change",
                        other,
                        self.sculk_actor(),
                        Some(now),
                    );
                    self.level_event_at(3003, other, 0);
                    self.wax_on_sound(at(hit.pos));
                }
                self.award_stat("used", &stack.item, 1);
                if !self.creative {
                    self.consume_held();
                }
                return Ok(UseResult::Success { swing: true });
            }
        }
        if item == "flint_and_steel" || item == "fire_charge" {
            if !self.may_build() && !self.can_place_on_in_adventure(hit.pos) {
                return Ok(UseResult::Fail);
            }
            if let Some(lit) = self.fire_starter_lit(&hit, stack, true)? {
                self.level_set(hit.pos, lit)?;
                let sound = if item == "fire_charge" {
                    "minecraft:item.firecharge.use"
                } else {
                    "minecraft:item.flintandsteel.use"
                };
                let pitch = sunrise_client::items::fire_starter_pitch(
                    item == "fire_charge",
                    &mut self.world.random,
                );
                self.play_block_use_sound(sound, at(hit.pos), pitch);
                // FlintAndSteelItem / FireChargeItem.useOn: BLOCK_CHANGE at the lit block,
                // and ItemStack.useOn's used statistic.
                self.post_block_game_event(
                    "minecraft:block_change",
                    hit.pos,
                    self.sculk_actor(),
                    None,
                );
                self.award_stat("used", &stack.item, 1);
                if item == "fire_charge" {
                    self.consume_held();
                } else {
                    self.damage_held(1);
                }
                return Ok(UseResult::Success { swing: true });
            }
            let target = [
                hit.pos[0] + hit.face[0],
                hit.pos[1] + hit.face[1],
                hit.pos[2] + hit.face[2],
            ];
            if self.can_place_used_fire(target) {
                // With mods, the fire goes in and `BaseFireBlock.onPlace`
                // fills the frame, as vanilla's order is, so that
                // `portal_spawn` is asked once, where vanilla asks it.
                if !mod_api::__private::mods().is_empty() || !self.try_light_portal(target)? {
                    self.place_used_fire(target)?;
                }
                let sound = sunrise_client::items::fire_starter_sound(item == "fire_charge");
                let pitch = sunrise_client::items::fire_starter_pitch(
                    item == "fire_charge",
                    &mut self.world.random,
                );
                self.play_block_use_sound(sound, at(target), pitch);
                // BLOCK_PLACE: flint and steel names the clicked block, a fire charge the
                // fire. Then ItemStack.useOn's used statistic.
                let placed_at = if item == "fire_charge" {
                    target
                } else {
                    hit.pos
                };
                self.post_block_game_event(
                    "minecraft:block_place",
                    placed_at,
                    self.sculk_actor(),
                    None,
                );
                self.award_stat("used", &stack.item, 1);
                if item == "fire_charge" {
                    self.consume_held();
                } else {
                    self.damage_held(1);
                }
                return Ok(UseResult::Success { swing: true });
            }
            return Ok(UseResult::Fail);
        }
        Ok(UseResult::Pass)
    }
    pub(super) fn can_place_used_fire(&mut self, target: [i32; 3]) -> bool {
        let pos = BlockPos::new(target[0], target[1], target[2]);
        let direction = match ((self.yaw / 90.0 + 0.5).floor() as i32).rem_euclid(4) {
            0 => Direction::South,
            1 => Direction::West,
            2 => Direction::North,
            _ => Direction::East,
        };
        self.with_level(|b, level, ticks| b.fire_can_be_placed_at(level, ticks, pos, direction))
    }
    fn place_used_fire(&mut self, target: [i32; 3]) -> Result<()> {
        let pos = BlockPos::new(target[0], target[1], target[2]);
        let state = self.with_level(|b, level, ticks| b.fire_state_for(level, ticks, pos));
        self.level_set(target, state)
    }
    /// `ExperienceOrb.followNearbyPlayer`'s candidates: each orb goes for the
    /// nearest living player within 8 blocks, the host's own or a guest. Each
    /// is the player's feet, the point the orb homes on and the player's UUID.
    pub(super) fn orb_followed(&self) -> Vec<xp_orb_targeting::OrbCandidate> {
        let spectator = self.spectator();
        let player = self
            .player
            .as_ref()
            .filter(|p| !p.health.dead_or_dying() && !spectator && !self.lan_acting_fake);
        // ExperienceOrb.followNearbyPlayer: each orb goes for the nearest
        // living player within 8 blocks, the host's own or a guest.
        let uuid = self
            .level
            .player_uuid
            .map(sunrise_client::lan::playerdata::ints_uuid);
        let mut followed: Vec<xp_orb_targeting::OrbCandidate> = player
            .map(|p| {
                let feet = [p.position.x, p.position.y, p.position.z];
                let eye = f64::from(p.dimensions.eye_height);
                (
                    feet,
                    [feet[0], feet[1] + eye / 2.0, feet[2]],
                    uuid,
                    p.entity_id as u32,
                )
            })
            .into_iter()
            .collect();
        if let Some(host) = self.lan.as_ref() {
            followed.extend(
                host.hub
                    .guests
                    .iter()
                    .filter(|g| {
                        g.placed
                            && g.participates_in_player_simulation()
                            && g.loaded
                            && !g.dead
                            && g.health > 0.0
                    })
                    .filter(|g| !g.spectator())
                    .map(|g| {
                        let p = g.position;
                        (
                            p,
                            [p[0], p[1] + g.eye_height() / 2.0, p[2]],
                            Some(g.uuid),
                            g.entity_id as u32,
                        )
                    }),
            );
        }
        followed
    }
    /// `Player.getVehicle().getBoundingBox()` for the local player's vehicle
    /// (`riding`): a mob, one of `object_target`'s kinds (boat, cart, armour
    /// stand, item frame, painting) or a tracked special — the same chain
    /// `guest_target_box` resolves a guest's vehicle through.
    fn riding_box(&self) -> Option<Aabb> {
        let id = self.riding?;
        if let Some(m) = self.mobs.iter().find(|m| m.id == id) {
            return Some(m.bounds());
        }
        if let Some((lo, hi, _)) = self.object_target(id as i32) {
            return Some(Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            ));
        }
        self.tracked_special_box(id as i32)
    }
    /// `Player.giveExperiencePoints`: the signed grant, zero included (the `player_xp_change` listener
    /// may rewrite it), changes the score before the XP bar.
    /// `add_experience_levels` runs once for each level gained or lost, and the mods hear
    /// of the level change once.
    pub(super) fn give_experience_points(&mut self, points: i32) {
        let Some(points) = self.xp_change(points) else {
            return;
        };
        self.player_score = self.player_score.wrapping_add(points);
        let old_level = self.experience.level;
        self.experience.add_points(points);
        while self.experience.progress < 0.0 {
            let xp = &self.experience;
            let lost = xp.progress * sunrise_client::entities::Experience::needed(xp.level) as f32;
            let had_level = xp.level > 0;
            self.add_experience_levels(-1);
            let xp = &mut self.experience;
            xp.progress = if had_level {
                1.0 + lost / sunrise_client::entities::Experience::needed(xp.level) as f32
            } else {
                0.0
            };
        }
        while self.experience.progress >= 1.0 {
            let xp = &mut self.experience;
            xp.progress =
                (xp.progress - 1.0) * sunrise_client::entities::Experience::needed(xp.level) as f32;
            self.add_experience_levels(1);
            let xp = &mut self.experience;
            xp.progress /= sunrise_client::entities::Experience::needed(xp.level) as f32;
        }
        self.native_player_level_changed(old_level);
    }
    /// Rebuild GPU-owned state after the backend reports device loss.
    fn recover_device(&mut self) -> Result<()> {
        if !self.renderer.as_ref().is_some_and(Renderer::device_lost) {
            return Ok(());
        }
        let now = Instant::now();
        if self
            .last_device_recovery
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(60))
        {
            return Err("the graphics device was lost again after recovery".into());
        }
        self.last_device_recovery = Some(now);
        let window = self
            .window
            .clone()
            .ok_or("no window for graphics recovery")?;
        log::warn!("Sunrise: rebuilding the renderer after graphics device loss");
        // Drop the old surface and device before opening another on this window.
        let mut old = self
            .renderer
            .take()
            .ok_or("no renderer for graphics recovery")?;
        // A GPU-culled frame may be what lost it: the renderers after this one don't
        // offer GPU culling, until Sunrise restarts (`graphics::lose_gpu_culling`).
        if old.gpu_frame() {
            log::warn!(
                "Sunrise: GPU culling off until the next start: the graphics device was lost in one of its frames"
            );
            sunrise_client::graphics::lose_gpu_culling();
        }
        let (screenshot, screenshot_result) = (old.screenshot.take(), old.screenshot_result.take());
        let display = old.display_reading();
        drop(old);
        // The new device may be smaller or larger than the lost one: stitch the GUI atlas for it
        // and rebind its UV consumers before the renderer uploads pages and anything publishes.
        let device = pollster::block_on(sunrise_client::gfx::Opened::for_window(
            &window,
            self.options.offscreen,
            self.settings.sunrise.api,
            self.pack.render.texture_layers,
        ))?;
        if self.gui.stitch_for_device(
            &self.pack.source,
            &self.vanilla_root(),
            device.max_atlas_size(),
        )? {
            self.particles.reload(
                &self.pack.source,
                &self.gui.atlas.sprites,
                &self.gui.atlas.translucent,
            );
            self.standalone_geometry = native_block_entity_draw::StandaloneGeometry::new(
                &self.pack,
                &self.gui,
                self.res.try_block_entity_render(),
            );
            sunrise_client::ui_theme::reset_atlas();
            self.native_texture_atlas_stitched(mod_api::TextureAtlas::Gui);
        }
        let (mut renderer, required) = build_renderer(
            window,
            self.options.offscreen,
            &self.pack,
            &self.gui,
            sunrise_client::gfx::RendererDevice::Opened(Box::new(device)),
            None,
            self.settings.sunrise.api,
            self.settings.sunrise.translucent_batching,
            self.effective_vsync(),
            &self.settings.sunrise,
            shader_packs::picked_pack_shape(&self.settings),
            display,
            self.settings.texture_atlas_settings(),
        )?;
        required?;
        (renderer.screenshot, renderer.screenshot_result) = (screenshot, screenshot_result);
        self.adopt_renderer(renderer);
        log::info!("Sunrise: graphics device recovered; rebuilding world meshes");
        Ok(())
    }
    /// A translated message for the player the game acts for: `overlay` is
    /// `sendOverlayMessage` (the action bar), else `sendSystemMessage` (chat).
    /// A guest gets a packet, so its own client translates it.
    fn tell_sleeper(&mut self, key: &str, fallback: &str, overlay: bool) {
        if self.lan_acting.is_some() {
            self.guest_packets
                .push(sunrise_protocol::packets::play::Clientbound::SystemChat(
                    sunrise_protocol::packets::play::SystemChat {
                        content: sunrise_protocol::text::translatable(key, &[]),
                        overlay,
                    },
                ));
        } else if overlay {
            self.message = Some((self.translate_with_fallback(key, fallback), Instant::now()));
        } else {
            self.system_message(sunrise_client::text::Text::translate(key, &[]));
        }
    }
    /// ServerPlayer.startSleepInBed: in reach, not obstructed, night or thunder,
    /// and (outside creative) no monsters within 8 blocks; the bed becomes the
    /// respawn point either way.
    fn try_sleep(&mut self, pos: [i32; 3]) {
        if self.fake_blocks(mod_api::FakeEffect::Sleep) {
            return;
        }
        let Some((position, yaw)) = self.player.as_ref().map(|p| (p.position, p.yaw)) else {
            return;
        };
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let props = self
            .pack
            .assets
            .registry
            .state(state)
            .map(|s| s.properties.clone())
            .unwrap_or_default();
        // The head and the foot: the block faces from the foot to the head.
        let d = facing_offset(props.get("facing").map_or("north", String::as_str));
        let (head, foot) = if props.get("part").map(String::as_str) == Some("foot") {
            ([pos[0] + d[0], pos[1], pos[2] + d[2]], pos)
        } else {
            (pos, [pos[0] - d[0], pos[1], pos[2] - d[2]])
        };
        // AbstractBedBlock.useWithoutItem: in an occupied bed a villager
        // asleep there is woken (kickVillagerOutOfBed), else someone else
        // sleeps in it; the player stays up either way.
        let head_state = self.world.block(head[0], head[1], head[2]);
        let occupied = self
            .pack
            .assets
            .registry
            .state(head_state)
            .is_some_and(|s| s.properties.get("occupied").is_some_and(|v| v == "true"));
        if occupied {
            if !self.kick_villager_out_of_bed(head) {
                self.tell_sleeper("block.minecraft.bed.occupied", "This bed is occupied", true);
            }
            return;
        }
        // ServerPlayer.bedInRange / isReachableBedBlock: within 3 blocks
        // sideways and 2 vertically of the head or the foot.
        let reachable = |bed: [i32; 3]| {
            (position.x - f64::from(bed[0]) - 0.5).abs() <= 3.0
                && (position.y - f64::from(bed[1])).abs() <= 2.0
                && (position.z - f64::from(bed[2]) - 0.5).abs() <= 3.0
        };
        let mut problem: Option<&'static str> = if !reachable(head) && !reachable(foot) {
            Some("too_far_away")
        } else {
            None
        };
        // ServerPlayer.bedBlocked: Player.freeAt (not suffocating) for the
        // blocks above the head and the foot, as `in_wall` reads suffocating.
        let blocked = problem.is_none()
            && [head, foot].iter().any(|bed| {
                let f = self
                    .res
                    .facts()
                    .get(self.world.block(bed[0], bed[1] + 1, bed[2]));
                f.full_collision && f.solid_render
            });
        if blocked {
            problem = Some("obstructed");
        }
        // In reach (bedInRange), day or night: setRespawnPosition(
        // RespawnConfig(the head, the player's yaw), not forced). It replaces
        // an anchor or a spawnpoint; respawning checks the bed is still there
        // and finds where to stand up (findRespawnAndUseSpawnBlock).
        // Straw beds never set it (`straw_bed_rule`).
        if problem.is_none() && self.bed_sets_spawn(state) {
            let config = respawn_anchor::RespawnConfig::wrapped(self.dim, head, yaw, false);
            // ServerPlayer.setRespawnPosition: the chat message only when the
            // point moved (`isSamePosition`).
            let moved = self
                .respawn_config
                .is_none_or(|c| (c.dim, c.pos) != (config.dim, config.pos));
            // `canSetSpawn` calls the setter every time, the same bed too (it
            // refreshes the angle).
            let cancel = self.spawn_point_change(self.dim, Some(head), false);
            if !cancel {
                self.respawn_config = Some(config);
                if moved {
                    self.tell_sleeper("block.minecraft.set_spawn", "Respawn point set", false);
                }
                // The fallback becomes the world spawn: a guest's is none of its
                // own, so it follows `setworldspawn`.
                self.spawn = if self.lan_acting.is_some() {
                    None
                } else {
                    self.level
                        .world_spawn
                        .map(|[x, y, z]| {
                            Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5)
                        })
                        .or(self.spawn)
                };
            }
        }
        if problem.is_none() && !self.bed_can_sleep_now() {
            problem = Some("not_possible_now");
        }
        // Creative players skip the monster check (`!this.isCreative()`).
        if problem.is_none()
            && !self.creative
            && self.mobs.iter().any(|m| {
                // `Monster.class` entities that `isPreventingPlayerRest`: a
                // zombified piglin only while it is angry.
                sunrise_client::mobs::ai::is_monster(m.species())
                    && (m.species() != sunrise_client::mobs::Species::ZombifiedPiglin
                        || (m.extra.anger > 0 && m.ai.target.is_some()))
                    && m.alive()
                    && (m.position.x - position.x).abs() < 8.0
                    && (m.position.y - position.y).abs() < 5.0
                    && (m.position.z - position.z).abs() < 8.0
            })
        {
            problem = Some("not_safe");
        }
        // NeoForge's CanPlayerSleepEvent: a mod may clear or replace the
        // problem vanilla found, or add one.
        let mut problem = problem.map(str::to_owned);
        if let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone()) {
            self.native_can_player_sleep(player, &name, head, &mut problem);
        }
        if let Some(problem) = problem {
            match problem.as_str() {
                "too_far_away" => self.tell_sleeper(
                    "block.minecraft.bed.too_far_away",
                    "You may not rest now; the bed is too far away",
                    true,
                ),
                "obstructed" => self.tell_sleeper(
                    "block.minecraft.bed.obstructed",
                    "This bed is obstructed",
                    true,
                ),
                "not_safe" => self.tell_sleeper(
                    "block.minecraft.bed.not_safe",
                    "You may not rest now; there are monsters nearby",
                    true,
                ),
                "not_possible_now" => self.tell_sleeper(
                    "block.minecraft.bed.no_sleep",
                    "You can sleep only at night or during thunderstorms",
                    true,
                ),
                // `other_problem` and any name a mod invents: refused without a message.
                _ => {}
            }
            return;
        }
        // ServerPlayer.startSleepInBed: a percentage above 100 cannot skip the night.
        if self
            .ui
            .rules
            .int_at(sunrise_client::gamerules::vanilla::PLAYERS_SLEEPING_PERCENTAGE)
            > 100
        {
            self.tell_sleeper(
                "sleep.not_possible",
                "No amount of rest can pass this night",
                true,
            );
        }
        if let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone()) {
            let mut cancel = false;
            self.native_player_bed_enter(player, &name, head, &mut cancel);
            if cancel {
                return;
            }
        }
        if self.lie_in_bed(head) {
            let bed = self.world.block(head[0], head[1], head[2]);
            self.award_custom_stat(self.slept_in_bed_stat(bed), 1);
            self.trigger_player("minecraft:slept_in_bed");
            self.stats.reset("custom", "minecraft:time_since_rest");
        }
    }
    /// `LivingEntity.startSleeping`: the player takes the bed whose head
    /// `head` names, moved onto it and asleep from now on. False when the
    /// bed gives no shape to lie on.
    fn lie_in_bed(&mut self, head: [i32; 3]) -> bool {
        // The shape the bed gives the player to lie on (`getSleepHeight`).
        // A straw bed's head answers with its foot's shape, and with no foot
        // of its own there is none, so the player stays up as
        // `startSleeping` returning false leaves them.
        let Some(at) = self.sleeping_position(head) else {
            return false;
        };
        // LivingEntity.startSleeping: stopRiding once the bed is valid.
        self.stop_riding();
        self.stop_mining();
        self.set_mapping_held("key.attack", false);
        self.set_mapping_held("key.use", false);
        self.using = None;
        self.sleeping = Some(head);
        self.sleep_timer = 0;
        // LivingEntity.startSleeping and ServerPlayer.startSleeping: setPosToBed
        // and connection.teleport.
        if let Some(p) = self.player.as_mut() {
            p.position = at;

            p.velocity = Vec3::ZERO;
            let _ = p.set_sleeping(true);
            p.pitch = 0.0;
            p.teleported = true;
            p.teleported_delta = [false; 3];
        }
        self.pitch = 0.0;
        if self.lan_acting.is_none() {
            self.previous_position = at;
            self.snap_player_model_pitch();
        }
        self.occupy_bed(head);
        true
    }
    /// `Player.stopSleepInBed`: preserve the flags of the owning wake caller.
    fn wake_up(&mut self, wake_immediately: bool, update_level: bool) {
        if self.remote.is_some() {
            // InBedChatScreen.sendWakeUp: the server wakes the player.
            self.remote_stop_sleeping();
            return;
        }
        // PlayerWakeUpEvent: heard at the start of stopSleepInBed, still in bed;
        // a listener may have woken the player, so the sleep is read after it.
        self.native_acting_bed_leave(wake_immediately, update_level);
        let Some(head) = self.sleeping.take() else {
            return;
        };
        self.sleep_timer = if wake_immediately { 0 } else { 100 };
        // The active guest's live body wakes here; the hub owns its wake
        // animation and sleeping metadata, not the suspended outer actor.
        if let Some(i) = self.lan_acting.and_then(|conn| self.guest_index(conn))
            && let Some(host) = self.lan.as_mut()
        {
            // The live turn may have changed its bed since entry.
            host.hub.guests.update(i, |guest| guest.bed = Some(head));
            host.hub.wake(i, wake_immediately);
            // This live body applies the stand-up pose below, not the later
            // wake_woken_guests pass over stored guest bodies.
            host.hub.guests.update(i, |guest| guest.woke_from = None);
        }
        let stand = self.bed_wake_pose(head, self.yaw);
        if let Some(p) = self.player.as_mut() {
            if let Some((at, yaw)) = stand {
                p.position = at;
                p.velocity = Vec3::ZERO;
                p.yaw = yaw;
                p.pitch = 0.0;
                p.teleported = true;
                p.teleported_delta = [false; 3];
            }
            let _ = p.set_sleeping(false);
        }
        if let Some((at, yaw)) = stand {
            self.yaw = yaw;
            self.pitch = 0.0;
            if self.lan_acting.is_none() {
                self.previous_position = at;
                self.snap_player_model_pitch();
            }
        }
    }
    pub(in crate::game) fn tick_sleep(&mut self) {
        self.sleep_timer = tick_sleep_timer(self.sleep_timer, self.sleeping.is_some());
        if let Some(head) = self.sleeping {
            // LivingEntity.tick's checkBedExists: a sleeper whose bed is
            // gone gets up.
            let bed = self.world.block(head[0], head[1], head[2]);
            let bed_gone = !self.res.facts().get(bed).name.ends_with("_bed");
            // Vanilla has no Shift wake-up: the chat screen's capitals must
            // not get the sleeper up.
            let wake_immediately = bed_gone
                || self
                    .player
                    .as_ref()
                    .is_none_or(|p| p.health.dead_or_dying());
            if wake_immediately
                || (self.screen != Screen::Chat && self.held_mapping(Mapping::Sneak))
            {
                self.wake_up(wake_immediately, true);
                return;
            }
            // setPosToBed: the bed's top (`getSleepHeight`) and 0.125 above
            // it. A sleeper whose bed no longer gives a shape to lie on stays
            // where it is: vanilla only ever places a sleeper when it starts.
            let at = self.sleeping_position(head);
            if let (Some(p), Some(at)) = (self.player.as_mut(), at) {
                p.position = at;
                p.velocity = Vec3::ZERO;
                p.pitch = 0.0;
            }
            // LivingEntity.tick: a sleeper's xRot is 0.
            self.pitch = 0.0;
            self.snap_player_model_pitch();
        }
        // ServerLevel.tick: once enough of the players have slept 100 ticks,
        // skip to morning if `advance_time` is on, wake everyone, and
        // resetWeatherCycle if `advance_weather` is on and it is raining,
        // unless a mod's `sleep_skip_night` cancels it.
        if self.sleep_skips_night() {
            if self
                .ui
                .rules
                .bool_at(sunrise_client::gamerules::vanilla::ADVANCE_TIME)
            {
                // SleepFinishedTimeEvent is the sleep clock's only decision.
                if let Some((clock, target)) = self.sleep_finished_move() {
                    self.set_command_clock_time(&clock, target);
                }
            }
            // Level.isRaining: the rain level is above 0.2.
            if self
                .ui
                .rules
                .bool_at(sunrise_client::gamerules::vanilla::ADVANCE_WEATHER)
                && self.weather_levels(1.0).0 > 0.2
            {
                self.world.weather.raining = false;
                self.world.weather.thundering = false;
                self.world.weather.rain_time = 0;
                self.world.weather.thunder_time = 0;
            }
            if self.sleeping.is_some() {
                self.wake_up(false, false);
            }
            self.wake_guests();
            // SleepStatus.removeAllSleepers: waking everyone is not announced.
            self.sleep_seen.1 = 0;
        }
    }
    /// Copy the live player and world state into the `Level` that gets saved.
    fn sync_world_level_data(&mut self) {
        // Level.write saves the game rules (game_rules.dat) with the rest.
        self.level.game_rules = self.ui.rules.by_name();
        // PrimaryLevelData.setDifficulty: /difficulty and the options button change it.
        self.level.difficulty =
            sunrise_client::title::DIFFICULTIES[usize::from(self.ui.difficulty) & 3].into();
        self.level.time = self.world.game_time;
        self.level.weather = self.world.weather;
        // level.dat holds the Overworld clock's total, whichever level is in place.
        self.level.day_time = self.command_clock_time("minecraft:overworld");
        self.level.end_clock = Some(self.world.end_clock);
    }

    pub(super) fn sync_level(&mut self) {
        self.sync_world_level_data();
        self.level.player_impulse_grace = self.player.as_ref().map(|p| p.impulse.reset_grace_time);
        self.level.player_fall_particles = self
            .player
            .as_ref()
            .map(|p| p.spawn_extra_particles_on_fall);
        // A server has no player of its own; the entry of `guest_turn` puts one in
        // `self.player` only while a command reads it.
        if self.player.is_none() && self.options.dedicated.is_some() {
            return;
        }
        self.level.selected_slot = self.selected as i32;
        let mode = self.mode_id();
        // The vehicle the player rides (`ServerPlayer.saveParentVehicle`).
        let others = self
            .lan
            .as_ref()
            .map(|h| self.ridden_by_others(&h.hub.guests, self.dim.id(), None, false))
            .unwrap_or_default();
        let vehicle = self.parent_vehicle_tag(self.riding, &others);
        // Only the local player's file is this one's to rewrite.
        let local = self.serving.is_none() && self.options.dedicated.is_none();
        let level = &mut self.level;
        level.creative = self.creative;
        level.game_mode = mode;
        level.previous_game_mode = self.previous_mode.map(mod_api::GameMode::id);
        level.spawn = self.spawn.map(|s| [s.x, s.y, s.z]);
        level.respawn_elsewhere = respawn_anchor::RespawnConfig::kept(
            level.respawn_elsewhere.take(),
            self.respawn_config,
        );
        level.rotation = [self.yaw, self.pitch];
        level.xp_level = self.experience.level;
        level.xp_progress = self.experience.progress;
        level.xp_total = self.experience.total;
        level.player_score = self.player_score;
        level.root_vehicle = vehicle.as_ref().map(|v| v.2).or(self.pending_vehicle);
        // The ride's tree, or none once the file's own came back.
        match vehicle {
            Some((tag, ..)) => {
                let extra = level
                    .player_extra
                    .get_or_insert_with(|| sunrise_core::nbt::Tag::Compound(Default::default()));
                if let sunrise_core::nbt::Tag::Compound(map) = extra {
                    map.insert("RootVehicle".into(), tag);
                }
            }
            None if local && self.pending_vehicle.is_none() => {
                if let Some(sunrise_core::nbt::Tag::Compound(map)) = &mut level.player_extra {
                    map.remove(&"RootVehicle".into());
                }
            }
            None => {}
        }
        if let Some(p) = &self.player {
            level.position = Some([p.position.x, p.position.y, p.position.z]);
            level.health = p.health.health();
            level.absorption = p.health.absorption();
            let extra = level
                .player_extra
                .get_or_insert_with(|| sunrise_core::nbt::Tag::Compound(Default::default()));
            if let sunrise_core::nbt::Tag::Compound(map) = extra {
                map.insert(
                    "Fire".into(),
                    sunrise_core::nbt::Tag::Short(
                        p.fire_ticks.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
                    ),
                );
                map.insert("Air".into(), sunrise_core::nbt::Tag::Short(p.air as i16));
                // Entity.saveWithoutId: the `Invulnerable` flag always, its
                // `invulnerable_time` window only while open, like a mob's
                // save (#916).
                map.insert(
                    "Invulnerable".into(),
                    sunrise_core::nbt::Tag::Byte(i8::from(p.health.permanently_invulnerable)),
                );
                if p.health.invulnerable_time > 0 {
                    map.insert(
                        "invulnerable_time".into(),
                        sunrise_core::nbt::Tag::Int(p.health.invulnerable_time),
                    );
                } else {
                    map.remove(&"invulnerable_time".into());
                }
                // LivingEntity.addAdditionalSaveData: AttributeMap.pack.
                map.insert("attributes".into(), p.saved_attributes());
            }
            level.fall_distance = p.fall_distance;

            level.food = p.health.food.food;
            level.saturation = p.health.food.saturation;
            level.flying = p.flying;
        }
        level.inventory = self
            .inventory
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                s.as_ref()
                    .map(|s| (i as u8, s.item.to_string(), s.count, s.damage))
            })
            .collect();
        level.inventory_components = self
            .inventory
            .iter()
            .enumerate()
            .filter_map(|(i, s)| Some((i as u8, s.as_ref()?.extra.clone()?)))
            .collect();
        if let Some(s) = &self.offhand {
            level
                .inventory
                .push((150, s.item.to_string(), s.count, s.damage));
            if let Some(extra) = s.extra.clone() {
                level.inventory_components.insert(150, extra);
            }
        }
        for (equipment_slot, s) in self.equipment.iter().enumerate() {
            if let (Some(s), Some(slot)) = (s, sunrise_client::save::saved_slot(equipment_slot)) {
                level
                    .inventory
                    .push((slot, s.item.to_string(), s.count, s.damage));
                if let Some(extra) = s.extra.clone() {
                    level.inventory_components.insert(slot, extra);
                }
            }
        }
        // Vanilla stores armor in slots 100 (feet) to 103 (head).
        for (i, s) in self.armor.iter().enumerate() {
            if let Some(s) = s {
                level
                    .inventory
                    .push((103 - i as u8, s.item.to_string(), s.count, s.damage));
                if let Some(extra) = s.extra.clone() {
                    level.inventory_components.insert(103 - i as u8, extra);
                }
            }
        }
        level.dimension = self.dim.id().to_owned();
        // An open ender chest holds the live copy.
        if let Some(key) = self.ender_open
            && let Some(BlockEntity::Chest(items)) = self.world.block_entities.get(&key)
        {
            self.ender_items = items.clone();
        }
        level.ender_items = self.ender_items.clone();
        level.active_effects = self.potions.effects.saved();
    }
    /// The saved player's hands, armor, body, saddle and ender chest go in
    /// their fields; the 36 inventory slots come back.
    fn load_saved_inventory(&mut self, level: &Level) -> Vec<Option<ItemStack>> {
        let mut inventory = vec![None; 36];
        self.armor = [None, None, None, None];
        self.offhand = None;
        self.equipment = Default::default();
        self.ender_items = if level.ender_items.len() == 27 {
            level.ender_items.clone()
        } else {
            vec![None; 27]
        };
        for (slot, id, count, damage) in &level.inventory {
            let stack = ItemStack {
                item: Arc::from(id.as_str()),
                count: *count,
                damage: *damage,
                extra: level.inventory_components.get(slot).cloned(),
            };
            if (100..104).contains(slot) {
                self.armor[usize::from(103 - slot)] = Some(stack);
            } else if *slot == 150 {
                self.offhand = Some(stack);
            } else if let Some(equipment_slot) = sunrise_client::save::equipment_slot(*slot) {
                self.equipment.set(equipment_slot, Some(stack));
            } else if let Some(cell) = inventory.get_mut(usize::from(*slot)) {
                *cell = Some(stack);
            }
        }
        inventory
    }
    fn render(&mut self) -> Result<bool> {
        if self.screenshot_sequence.is_some() {
            return self.render_sequence_frame();
        }
        self.render_frame()
    }
    /// Render the frame, then its two moments after that: `render_frame_post` whatever
    /// became of it, `flip_frame` once it was presented.
    fn render_and_flip(&mut self, elapsed: Duration) -> Result<bool> {
        let rendered = match self.render() {
            // The callback can fire during this frame; rebuild on
            // the next redraw rather than exiting on its error.
            Err(_) if self.renderer.as_ref().is_some_and(Renderer::device_lost) => false,
            result => result?,
        };
        self.finish_render_frame(elapsed);
        if rendered {
            // The submit thread may still hold the frame: a listener hears it after the
            // present, and with none the next frame keeps overlapping it.
            if mod_api::__private::listened(mod_api::HookId::FlipFrame)
                && let Some(renderer) = &mut self.renderer
            {
                renderer.wait_submissions();
                if renderer.device_lost() {
                    return Ok(false);
                }
            }
            self.native_flip_frame();
        }
        Ok(rendered)
    }
    fn render_frame(&mut self) -> Result<bool> {
        self.render_with_hooks(|renderer, frame, hooks| {
            sunrise_client::spans::time("renderer", || renderer.render(frame, hooks))
        })
    }

    /// Build the complete frame before handing its borrowed geometry and lighting to
    /// the consumer. Tests use the same boundary as GPU submission.
    #[cfg(test)]
    fn render_with(
        &mut self,
        consume: impl FnOnce(&mut Renderer, &Frame<'_>) -> Result<bool>,
    ) -> Result<bool> {
        self.render_with_hooks(|renderer, frame, _| consume(renderer, frame))
    }

    fn render_with_hooks(
        &mut self,
        consume: impl FnOnce(
            &mut Renderer,
            &Frame<'_>,
            Option<&mut dyn sunrise_client::gfx::world_primitives::FrameHooks>,
        ) -> Result<bool>,
    ) -> Result<bool> {
        self.begin_frame_draws();
        let prepare = spans::guard("prepare");
        let mut pr = spans::laps();
        // The window's GPU holds the atlas: its copy in memory is kept
        // compressed (a no-op once it is).
        if self.renderer.is_some() {
            self.gui.atlas.pack_pixels();
            self.pack.render.pixels.pack();
        }
        // Before any text of this frame: a full glyph region starts over.
        self.gui.atlas.font.begin_frame();
        if self.clock.still() {
            // Obfuscated text picks the same glyphs in a frame of the same world tick.
            self.gui.atlas.font.reseed(self.visual_ticks() as i64);
        }
        self.glint_frame = self.capture_glint_frame();
        if self.renderer.as_mut().is_some_and(|r| r.display_changed()) {
            self.apply_ui_settings();
        }
        self.follow_frame();
        self.observe_screen();
        // `Gui.setScreen` clears the focus of the screen it leaves: a menu
        // that comes back starts with no widget highlighted.
        self.drop_left_screen_focus();
        // Minecraft.doWorldLoad: the LevelLoadingScreen while single player's
        // server starts, with the server's chunk map and progress.
        if let Some(starting) = &self.integrated_starting {
            // WorldOpenFlows.upgradeAndOpenWorld: the server's file fixers run on
            // FileFixerProgressScreen, before the LevelLoadingScreen has progress.
            let upgrade = starting.handle.upgrade();
            if upgrade.is_file_fixing() {
                sunrise_client::widgets::begin_screen(|| {
                    sunrise_client::widgets::ScreenInfo::new(
                        "file_fixer",
                        screen_info::tr("upgradeWorld.title", &[]),
                    )
                });
                let cursor = self
                    .renderer
                    .as_ref()
                    .and_then(|r| self.project_window_cursor([r.config.width, r.config.height]))
                    .map_or(self.cursor, |p| p.pixel());
                if let Some(renderer) = &mut self.renderer {
                    let size = [renderer.config.width, renderer.config.height];
                    let mut hud = HudBatch::new(size, self.settings.gui_scale_inputs());
                    sunrise_client::level_loading::draw_file_fixer(
                        &mut hud,
                        &self.gui.atlas,
                        upgrade,
                        cursor,
                    );
                    pr.lap("pr_loading");
                    drop(prepare);
                    return renderer.render(&loading_frame(&hud), None);
                }
                return Ok(false);
            }
            // The server's progress once it has some (`hasProgress`).
            let progress = Some(starting.handle.load_view())
                .filter(|v| v.has_progress())
                .map(|v| v.server_progress());
            sunrise_client::widgets::begin_screen(|| {
                screen_info::level_loading(progress, &screen_info::tr)
            });
            if let Some(renderer) = &mut self.renderer {
                let mut hud = HudBatch::new(
                    [renderer.config.width, renderer.config.height],
                    self.settings.gui_scale_inputs(),
                );
                let (w, h) = (hud.width, hud.height);
                sunrise_client::ui::menu_background(
                    &mut hud,
                    &self.gui.atlas,
                    0.0,
                    0.0,
                    w,
                    h,
                    false,
                );
                // The chunk map and progress are the server thread's.
                let view = self
                    .integrated_starting
                    .as_ref()
                    .map(|s| s.handle.load_view().clone());
                let status = |x: i32, z: i32| view.as_ref().and_then(|v| v.status(x, z));
                let progress = view
                    .as_ref()
                    .is_some_and(|v| v.has_progress())
                    .then_some(self.loading_progress);
                sunrise_client::level_loading::draw(
                    &mut hud,
                    &self.gui.atlas,
                    Some(&status),
                    progress,
                );
                pr.lap("pr_loading");
                drop(prepare);
                return renderer.render(&loading_frame(&hud), None);
            }
            return Ok(false);
        }
        if self.pending_entry.is_some() {
            if let (Some(renderer), Some(loader)) = (&mut self.renderer, &self.rest_loader) {
                sunrise_client::widgets::begin_screen(|| {
                    screen_info::level_loading(Some(loader.progress.fraction()), &screen_info::tr)
                });
                let mut hud = HudBatch::new(
                    [renderer.config.width, renderer.config.height],
                    self.settings.gui_scale_inputs(),
                );
                sunrise_client::level_loading::draw_progress(
                    &mut hud,
                    &self.gui.atlas,
                    loader.progress.line(),
                    loader.progress.fraction(),
                );
                pr.lap("pr_loading");
                drop(prepare);
                return renderer.render(&loading_frame(&hud), None);
            }
            return Ok(false);
        }
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return Ok(false);
        };
        // An opaque LoadingOverlay needs no world or screen behind it; the
        // ToastManager still draws in front of it.
        let reload_overlay = self.reload_overlay_frame();
        if let Some((bar, fade)) = reload_overlay.filter(|(_, fade)| fade.opaque()) {
            let mut hud = HudBatch::new(size, self.settings.gui_scale_inputs());
            self.draw_reload_overlay(&mut hud, bar, fade);
            self.draw_every_toast(&mut hud);
            self.pack.item_renderer.end_gui_frame();
            pr.lap("pr_loading");
            drop(prepare);
            return self
                .renderer
                .as_mut()
                .expect("renderer sized above")
                .render(&loading_frame(&hud), None);
        }
        pr.lap("pr_setup");
        let alpha = self.frame_world_partial_tick();
        self.refresh_client_pick(alpha as f32);
        // Camera.getCameraEntityPartialTicks: only players and their mounts stay unfrozen.
        let camera_alpha = self.camera_partial_tick();
        let view = self.frame_camera(camera_alpha as f32);
        self.check_screenshot_sequence_camera(view);
        // What faces the camera (particles, item sprites, block entity text) faces the frame's
        // camera (including mod angle overrides, roll, sleeping, or a spectated entity).
        self.posed_rotation = view.map(|(_, yaw, pitch, roll)| (yaw, pitch, roll));
        let eye = view.map(|_| {
            (
                glam::DVec3::from_array(self.view.eye),
                self.view.eye_rotation,
            )
        });
        let (camera, underwater, view_yaw, view_pitch, view_roll) = match view {
            Some((c, yaw, pitch, roll)) => {
                let s =
                    self.world
                        .block(c.x.floor() as i32, c.y.floor() as i32, c.z.floor() as i32);
                (
                    c,
                    self.res.facts().get(s).fluid == Fluid::Water,
                    yaw,
                    pitch,
                    roll,
                )
            }
            None => (
                glam::DVec3::new(0.5, 100.0, 0.5),
                false,
                self.yaw,
                self.pitch,
                0.0,
            ),
        };
        pr.lap("pr_camera");
        // `SoundEngine.updateSource`: use the actual camera basis, not the entity's own look
        // or presentation-only bob/spin transforms.
        let (forward, up) = if view.is_some() {
            (self.view.forwards, self.view.up)
        } else {
            let q = sunrise_client::camera::rotation_with_roll(view_yaw, view_pitch, view_roll);
            (q.transform([0.0, 0.0, -1.0]), q.transform([0.0, 1.0, 0.0]))
        };
        self.audio.set_listener(camera.to_array(), forward, up);
        self.follow_audio_stream_entities();
        pr.lap("pr_listener");
        let block = [
            sunrise_core::mth::floor(camera.x),
            sunrise_core::mth::floor(camera.y),
            sunrise_core::mth::floor(camera.z),
        ];
        // Camera.extractRenderState: only spectator inside a solidRender block disables smartCull.
        let cave_culling = self.settings.sunrise.cave_culling
            && !(self.spectator()
                && self
                    .res
                    .facts()
                    .get(self.world.block(block[0], block[1], block[2]))
                    .solid_render);
        // The bounded section graph runs on its worker while the frame is prepared, even when
        // smart culling is off (`SectionOcclusionGraph.runUpdates`).
        let effective_render_distance = self.effective_render_distance();
        let effective_detail_distance = (effective_render_distance * 16) as f32;
        if self.title.is_none()
            && let Some(renderer) = self.renderer.as_mut()
        {
            renderer.sync_loaded_columns(&self.world.chunks);
            sunrise_client::spans::time("occlusion", || {
                renderer.start_occlusion(camera, effective_detail_distance, cave_culling)
            });
        }
        pr.lap("pr_occlusion");
        let biome = self
            .world
            .chunks
            .get(&(block[0] >> 4, block[2] >> 4))
            .map(|c| c.biome(block[0], block[1], block[2]))
            .unwrap_or(0);
        // Asked before the environment's borrow: the renderer's pack shaders answer.
        let pack_expressions = self.reads_pack_expressions();
        // What the packs' expressions read, taken for the frame and put back after the post
        // chains: its strings are rewritten in place, so it allocates nothing once they have
        // grown to the dimension's and biome's names.
        let mut frame = std::mem::take(&mut self.expression_frame);
        let environment = self.view_environment();
        // EnvironmentAttributeProbe: the spatially interpolated biome colours,
        // Gaussian-weighted over the noise biomes around the camera (plains
        // where no chunk is loaded, as ClientLevel.getUncachedNoiseBiome).
        let biomes = world_biomes!(self);
        let weights = {
            let chunks = &self.world.chunks;
            let plains = biomes.id("minecraft:plains");
            sunrise_client::biome::gaussian_weights(camera.to_array(), |qx, qy, qz| {
                chunks
                    .get(&(qx >> 2, qz >> 2))
                    .map_or(plains, |c| c.biome(qx * 4, qy * 4, qz * 4))
            })
        };
        // visual/water_fog_color: no dimension sets it, its default is #050533.
        let water_fog = sunrise_client::biome::blend_rgb(&weights, |b| {
            sunrise_client::biome::rgb(biomes.get(b).water_fog)
        });
        // The attribute stack, with the camera's biome mix in the probe's order; the biome
        // maps as the packs left them.
        let attributes_of = |b: u16| {
            let info = biomes.get(b);
            environment.biome(&info.name).unwrap_or(&info.attributes)
        };
        let mut blend = sunrise_world::environment_system::BiomeBlend::default();
        for &(b, weight) in &weights {
            blend.accumulate(weight, attributes_of(b));
        }
        // The packs' biome post-process group, mixed the same way where they set one.
        let empty = sunrise_world::environment_attributes::AttributeMap::default();
        let post_of = |b: u16| {
            environment
                .biome_post(&biomes.get(b).name)
                .unwrap_or(&empty)
        };
        let mut post_blend = sunrise_world::environment_system::BiomeBlend::default();
        if environment.has_biome_post() {
            for &(b, weight) in &weights {
                post_blend.accumulate(weight, post_of(b));
            }
        }
        let (rain, thunder) = self.view_weather(alpha as f32);
        // What the packs' attribute expressions read (only a pack's stack has any).
        let mut polytone_frame = None;
        if self.pack.environments.is_some()
            || self.pack.polytone_particles.is_some()
            || self.pack.polytone_post.is_some()
            || pack_expressions
        {
            let light = self.world.light(block[0], block[1], block[2]);
            let info = biomes.get(biome);
            // Every field named, so a new one has to be written here too.
            let sunrise_client::environment::Frame {
                rain: frame_rain,
                thunder: frame_thunder,
                game_time,
                day_time,
                render_distance,
                sky_type,
                dimension,
                player,
                player_velocity,
                player_on_ground,
                in_water,
                camera: frame_camera,
                sky_light,
                block_light,
                biome: biome_name,
                temperature,
                downfall,
            } = &mut frame;
            dimension.clear();
            dimension.push_str(self.dim.id());
            biome_name.clone_from(&info.name);
            (*temperature, *downfall) = (info.temperature, info.downfall);
            (*frame_rain, *frame_thunder) = (rain, thunder);
            (*game_time, *day_time) = (self.world.game_time, self.view_day_time());
            *render_distance = effective_render_distance;
            *sky_type = match self.dimension_type().skybox {
                sunrise_world::dimension_type::Skybox::Overworld => 1,
                sunrise_world::dimension_type::Skybox::End => 2,
                sunrise_world::dimension_type::Skybox::None => 0,
            };
            *player = self.player.as_ref().map_or(camera.to_array(), |p| {
                [p.position.x, p.position.y, p.position.z]
            });
            *player_velocity = self
                .player
                .as_ref()
                .map_or([0.0; 3], |p| [p.velocity.x, p.velocity.y, p.velocity.z]);
            *player_on_ground = self.player.as_ref().is_some_and(|p| p.ground.on_ground);
            *in_water = underwater;
            *frame_camera = camera.to_array();
            (*sky_light, *block_light) = (i32::from(light >> 4), i32::from(light & 15));
            if let Some(particles) = &self.pack.polytone_particles
                && let Ok(mut f) = particles.frame.write()
            {
                f.clone_from(&frame);
            }
            environment.set_frame(&frame);
            polytone_frame = Some(&frame);
        }
        let fixed_time = self
            .settings
            .extras
            .pvp
            .tweaks
            .time
            .filter(|_| self.own_view())
            .map(|_| self.view_day_time());
        let sample_clock = |clock: &str| {
            if let Some(t) = fixed_time {
                t
            } else if let Some(remote) = self.remote.as_ref() {
                remote.named_clock_time(clock)
            } else {
                self.command_clock_time(clock)
            }
        };
        let mut sky = environment.sample_with_clock(
            &sunrise_client::environment::Sample {
                day_time: self.view_day_time(),
                rain,
                thunder,
                blend: Some(&blend),
                biome: Some(attributes_of(biome)),
                post_blend: Some(&post_blend),
                post_biome: Some(post_of(biome)),
            },
            &sample_clock,
        );
        self.end_flash_sky(&mut sky, alpha);
        if self.flash > 0 && !self.settings.file.bool("hideLightningFlashes") {
            // ClientLevel's flash layer while skyFlashTime > 0: the sky colour
            // moves 22% of the way to (0.8, 0.8, 1.0).
            let flash = [0.8, 0.8, 1.0];
            sky.sky_color =
                std::array::from_fn(|i| sky.sky_color[i] + (flash[i] - sky.sky_color[i]) * 0.22);
            sky.sky_light_factor = 1.0;
        }
        pr.lap("pr_environment");
        // FishingHookRenderer's line, then DebugRenderer: chunk borders and
        // hitboxes (F3+G, F3+B).
        let mut lines = self.fishing_line(alpha, size);
        let mut g = self.debug_gizmos(alpha as f32, camera.to_array());
        self.studio_camera_path(&mut g);
        lines.extend(g.lines);
        let on_top = g.on_top;
        pr.lap("pr_debug");
        let visuals = if self.title.is_none() {
            self.native_frame_visuals(alpha, &mut lines)
        } else {
            native_visuals::FrameVisuals::default()
        };
        pr.lap("pr_native_visuals");
        let view = if view_roll == 0.0 {
            sunrise_client::camera::view_matrix(view_yaw, view_pitch)
        } else {
            sunrise_client::camera::view_matrix_rolled(view_yaw, view_pitch, view_roll)
        };
        let at = camera.to_array();
        // GameRenderer.render3dHud: the debug crosshair, first person with the HUD shown.
        let mut crosshair = sunrise_client::gizmos::Gizmos::default();
        if debug_crosshair_drawn(
            self.debug_crosshair(),
            self.camera_type.is_first_person(),
            self.ui.hide_gui,
            self.shows_panorama(),
            self.overlay_holds_view(),
        ) {
            let scale = HudBatch::new(size, self.settings.gui_scale_inputs()).scale;
            crosshair.debug_crosshair(at, view_pitch, view_yaw, scale);
        }
        let debug_lines = sunrise_client::gizmos::camera_relative(&lines, at, view);
        let debug_lines_on_top = sunrise_client::gizmos::camera_relative(&on_top, at, view);
        let debug_crosshair = sunrise_client::gizmos::camera_relative(&crosshair.on_top, at, view);
        // LevelExtractor.extractBlockOutline: model translucency controls draw order;
        // no outline when an entity is targeted (Minecraft.hitResult an EntityHitResult).
        let mut outline_after_terrain = false;
        // GameRenderer.shouldRenderBlockOutline: not with the HUD hidden (F1), and only for a
        // player who may build. Adventure players only get one with a CAN_BREAK or
        // CAN_PLACE_ON item matching the block, and spectators with a menu block; Sunrise
        // does not open menus for spectators, so those never show it. An open
        // screen does not hide it: vanilla checks the HUD and the game mode, not the screen.
        let outline: Vec<_> = self
            .block_outline_hit()
            .filter(|h| {
                let state = self.world.block(h.pos[0], h.pos[1], h.pos[2]);
                let mut canceled = false;
                self.native_block_highlight(h.pos, state, alpha as f32, &mut canceled);
                !canceled
            })
            .map(|h| {
                let state = self.world.block(h.pos[0], h.pos[1], h.pos[2]);
                outline_after_terrain = self
                    .pack
                    .render
                    .model_catalog
                    .collect(state, BlockPos::new(h.pos[0], h.pos[1], h.pos[2]))
                    .is_ok_and(|models| {
                        models
                            .iter()
                            .any(|(_, model)| model.faces.iter().any(|f| f.translucent))
                    });
                self.outline_edges_iter(state, h.pos).collect()
            })
            .unwrap_or_default();
        pr.lap("pr_outline");
        let title = self.shows_panorama();
        let (yaw, pitch, roll) = if title {
            // PanoramaRenderer: the title cube spins at 0.1 degrees per tick, pitched 10 degrees.
            (
                self.panorama_rotation.update(
                    self.started.elapsed().as_secs_f32(),
                    self.settings.file.double("panoramaScrollSpeed") as f32,
                ),
                10.0,
                0.0,
            )
        } else {
            (view_yaw, view_pitch, view_roll)
        };
        let frame_view_matrix = if title {
            sunrise_client::camera::view_matrix(yaw, pitch)
        } else {
            view
        };
        let fov_degrees = if title {
            85.0
        } else {
            self.fov_degrees(camera_alpha as f32, camera.to_array())
        };
        let cull_fov_degrees = if title {
            85.0
        } else {
            sunrise_client::gfx::cull_fov(fov_degrees, (self.settings.fov as i32) as f32)
        };
        let camera_effects = fov::camera_effects_enabled(title, self.studio.flying());
        let bob = if !camera_effects {
            glam::Mat4::IDENTITY
        } else {
            self.bob_matrix(camera_alpha as f32)
        };
        let spin = if !camera_effects {
            glam::Mat4::IDENTITY
        } else {
            self.spinning_matrix(alpha as f32)
        };
        let terrain_distance = self.terrain_distance();
        let distance = (terrain_distance * 16) as f32;
        let depth_far =
            sunrise_client::camera::depth_far(terrain_distance, self.settings.cloud_range);
        let view_transform = if camera_effects
            && self.player.is_some()
            && mod_api::__private::listened(mod_api::HookId::RenderViewTransform)
            && let Some(renderer) = &self.renderer
        {
            let usable = renderer.view_transform_validator(
                [yaw, pitch, roll],
                fov_degrees,
                depth_far,
                bob,
                spin,
            );
            mod_api::ViewTransform::with_frame(usable, |transform| {
                self.native_render_view_transform(camera_alpha as f32, transform);
                glam::Mat4::from_cols_array(&transform.matrix())
            })
        } else {
            glam::Mat4::IDENTITY
        };
        // The pose the camera, GUI and hand see: the vanilla bob, then the mod's transform.
        let posed_bob = bob * view_transform;
        self.screen_blur_requested.set(false);
        // GUI and world hooks read the same frame snapshot. Keep its dimension allocation
        // between frames rather than formatting another copy for the world renderer.
        self.native_camera_entity_eye =
            (!title && self.camera_subject().is_some()).then_some(self.view.eye);
        let previous_camera = self.native_camera_frame.take();
        self.native_camera_frame = (!title && !mod_api::__private::mods().is_empty()).then(|| {
            let mut dimension =
                previous_camera.map_or_else(String::new, |snapshot| snapshot.dimension);
            dimension.clear();
            dimension.push_str(self.dim.id());
            let partial = camera_alpha as f32;
            mod_api::Camera {
                position: camera.to_array(),
                eye_height: self.view.eye_height_old
                    + (self.view.eye_height - self.view.eye_height_old) * partial,
                partial_tick: partial,
                yaw,
                pitch,
                roll,
                fov_degrees,
                near: 0.05,
                far: depth_far,
                view: (posed_bob * spin * frame_view_matrix).to_cols_array(),
                projection: glam::Mat4::perspective_rh(
                    fov_degrees.to_radians(),
                    size[0] as f32 / size[1].max(1) as f32,
                    depth_far,
                    0.05,
                )
                .to_cols_array(),
                dimension,
            }
        });
        let _projection_frame = (!title).then(|| self.activate_projection_frame());
        pr.lap("pr_projection");
        let vanilla_skin = self.apply_player_skin_override(title);
        // GUI hooks project against this frame's camera, including its bob/spin and FOV.
        let mut hud = sunrise_client::spans::time("hud", || {
            self.extract_frame_hud(
                size,
                reload_overlay,
                title,
                camera.to_array(),
                view_yaw,
                view_pitch,
                fov_degrees,
            )
        });
        if reload_overlay.is_none() {
            self.flush_ime(&mut hud);
        }
        self.resolve_inline_profiles(&hud.inline_profiles);
        pr.lap("pr_hud");
        // The packs' Polytone post chains, after vanilla's (#11486).
        let game_ticks = self.world.game_time as f64 + alpha;
        let delta_ticks = (game_ticks - self.polytone_post_ticks).clamp(0.0, 20.0) as f32;
        self.polytone_post_ticks = game_ticks;
        self.evaluate_pack_expressions(polytone_frame);
        let (polytone_effects, polytone_extras) = match (&self.pack.polytone_post, polytone_frame) {
            (Some(post), Some(frame)) => {
                let (effects, extras) = post.frame(frame);
                (effects, Some(extras))
            }
            _ => (Vec::new(), None),
        };
        let mut requested_post_effects = std::mem::take(&mut self.requested_post_effects_buf);
        self.sync_requested_post_effects(
            &mut requested_post_effects,
            polytone_effects.iter().map(String::as_str),
        );
        // Polytone's globals (wiki "Extra Shader Uniforms"), built only when something reads
        // them: here when an installed pack core shader does, and by the post effects when a
        // pass of theirs does (`Request::poly_globals`, called at most once a frame). The
        // renderer keeps the bytes in each reading core pipeline's blocks, which write them to
        // the GPU when that pipeline uploads its blocks for its draw; a post pass writes them
        // when it draws.
        let (sun_angle, day_time) = (sky.sun_angle, self.view_day_time());
        let player = (self.player.as_ref()).map_or(camera.to_array(), |p| {
            [p.position.x, p.position.y, p.position.z]
        });
        let poly_globals = move || sunrise_client::polytone_post::PolyGlobals {
            // 26.3's reverse-Z projection, as the world's (and its depth) is.
            projection: glam::Mat4::perspective_rh(
                fov_degrees.to_radians(),
                size[0] as f32 / size[1].max(1) as f32,
                depth_far,
                0.05,
            ),
            model_view: posed_bob * spin * frame_view_matrix,
            sun_angle: sun_angle.to_radians() - std::f32::consts::FRAC_PI_2,
            day_time: day_time.rem_euclid(24000) as f32,
            delta_time: delta_ticks,
            player,
        };
        let post_globals = move || poly_globals().bytes();
        let core_globals = self.reads_poly_globals().then(poly_globals);
        self.give_poly_globals(core_globals.as_ref());
        self.expression_frame = frame;
        // Camera.createProjectionMatrixForCulling: max(fov, configuredFov), without bob or spin,
        // under the mod's view transform.
        self.view_frustum = self.world_view_frustum(
            camera,
            [cull_fov_degrees, yaw, pitch, roll],
            view_transform,
            depth_far,
        );
        let billboard_frustum = if self.settings.sunrise.taa {
            self.renderer.as_ref().map(|r| {
                r.camera_frustum_with_jitter(
                    cull_fov_degrees,
                    yaw,
                    pitch,
                    roll,
                    view_transform,
                    depth_far,
                )
            })
        } else {
            self.view_frustum
        };
        // Keep the buffer local while builders need mutable access to the game.
        let mut entities = std::mem::take(&mut self.entity_vertices);
        entities.clear();
        let mut static_block_entities = std::sync::Arc::from([]);
        let mut visible_static_block_entity_sections = Vec::new();
        let mut static_block_entity_stats = StaticCacheFrameStats::default();
        self.see_through_vertices.clear();
        self.glowing_vertices.get_mut().clear();
        self.shadow_only_vertices.get_mut().clear();
        pr.lap("pr_entity_setup");
        // The pack is chosen before entity geometry, so a pack that starts this frame
        // keeps block-entity models in the CPU stream instead of dropping them.
        let mut pack_inputs = if title {
            Default::default()
        } else {
            self.shader_pack_inputs(camera, eye.map_or(camera, |e| e.0), alpha as f32, &sky)
        };
        // Retain the existing additive particle stream; the ordered dragon emitter appends
        // the eyes and rays to it without changing their material routing.
        let mut additive = self.particles.take_additive();
        if !title {
            self.frame_batch_reserved_quads.set(additive.len() / 4);
            let (built, static_sections, visible_static_sections, static_stats) =
                spans::time("entity_geometry", || {
                    self.entity_geometry(camera, alpha, billboard_frustum, entities, &mut additive)
                });
            entities = built;
            static_block_entities = static_sections;
            visible_static_block_entity_sections = visible_static_sections;
            static_block_entity_stats = static_stats;
        } else {
            self.frame_batch_reserved_quads.set(0);
            // No level behind the panorama, so no entity either: the GPU path's
            // draws are started by `begin_skinning`, which only `entity_geometry`
            // runs, and would otherwise stay those of the last world frame and
            // be drawn over the title (#8192). Its models stay stored.
            self.skin_frame.get_mut().clear_draws();
        }
        pr.lap("pr_entities");
        let bolts = if title {
            Vec::new()
        } else {
            self.bolt_instances(camera)
        };
        if !title {
            self.dragon_fireball_geometry(camera, alpha, &mut entities);
        }
        pr.lap("pr_dragon_fireballs_bolts");
        let sky_effect_blocked = self.camera_subject().is_some_and(|c| c.blocks_sky);
        let (celestial, celestial_bodies, stars) = if title {
            (Vec::new(), Vec::new(), None)
        } else if self.overworld_skybox() && !sky_effect_blocked {
            // Camera.doesMobEffectBlockSky: no sun, moon or stars.
            self.celestial_geometry(&sky, rain)
        } else if self.end_skybox() && self.settings.sunrise.sky {
            let (flash, bodies) = self.end_flash_geometry();
            (flash, bodies, None)
        } else {
            (Vec::new(), Vec::new(), None)
        };
        self.custom_sky_frame([camera.x, camera.y, camera.z], &sky, (rain, thunder), title);
        pr.lap("pr_celestial");
        let (mut hand, hand_items) = if title || self.scoping() {
            Default::default()
        } else {
            self.frame_batch_reserved_quads
                .set((additive.len() / 4).saturating_add(entities.len() / 4));
            self.hand_geometry(camera_alpha)
        };
        // GameRenderer.render3dHud: the screen effects after the hand, in
        // its pass (the hand pass draws with `bob * view`).
        if !title {
            let view = posed_bob * frame_view_matrix;
            let aspect = size[0] as f32 / size[1].max(1) as f32;
            self.item_activation_geometry(camera_alpha as f32, view, aspect, &mut hand);
        }
        pr.lap("pr_hand");
        let hud_fov = self.hud_fov_degrees(camera_alpha as f32, camera.to_array());
        let screen = if title {
            Vec::new()
        } else {
            let scale = self
                .player
                .as_mut()
                .and_then(|p| p.attribute("minecraft:scale").ok())
                .unwrap_or(1.0) as f32;
            let hand_projection = sunrise_client::camera::hand_projection(
                hud_fov,
                size[0] as f32,
                size[1].max(1) as f32,
                depth_far,
            );
            self.screen_quads(alpha as f32, size, scale, hand_projection)
        };
        if !title {
            self.eye_geometry(camera, &mut entities);
        }
        self.outline_fill(&outline, camera, &mut entities);
        pr.lap("pr_screen_end");
        let weather = if !title && self.can_have_weather() && self.settings.sunrise.weather {
            sunrise_client::spans::time("weather", || self.weather_geometry(camera, alpha))
        } else {
            Vec::new()
        };
        let border = (!title)
            .then(|| self.border_draw(camera.to_array(), alpha as f32))
            .flatten();
        pr.lap("pr_weather_border");
        if title {
            self.dynamic_lights.clear();
        } else {
            self.gather_dynamic_lights(camera, alpha);
        }
        pr.lap("pr_lights");
        pack_inputs.frame_timing = self
            .screenshot_sequence
            .as_ref()
            .map(screenshot_sequence::Sequence::timing);
        pr.lap("pr_shader_inputs");
        // Fog hooks may clear particles too. They must run before the final clear
        // boundary, not while constructing Frame after geometry has been extracted.
        let fog = self.frame_fog(camera, &sky, biome, water_fog);
        let mut explicit_particle_quads = std::mem::take(&mut self.explicit_particle_quads);
        explicit_particle_quads.clear();
        if !title {
            // The renderer decides on the packs' GPU particle shaders before the quads are
            // built, and takes the records of the types it draws after.
            self.polytone_gpu_decide(camera);
            let cull = self.particles.has_provider_particles().then(|| {
                sunrise_client::particles::ParticleCullView::new(
                    camera,
                    [yaw, pitch],
                    size,
                    self.settings.fov,
                    fov_degrees,
                    depth_far,
                )
            });
            self.client_particle_geometry(
                camera,
                alpha,
                fov_degrees,
                cull.as_ref()
                    .map(|cull| (cull, &mut explicit_particle_quads)),
                &mut entities,
            );
            self.polytone_gpu_particles(camera, alpha);
            self.follow_debug_selection();
        }
        let mut world_geometry = self.world_geometry.take().expect("view storage");
        let white = self.gui.atlas.sprites["sunrise/white"];
        world_geometry.reset(
            camera.to_array(),
            view,
            [(white.u0 + white.u1) * 0.5, (white.v0 + white.v1) * 0.5],
        );
        world_geometry.set_model_cut(entities.len() as u32);
        // Seconds of game time, wrapping every hour (72,000 ticks) so f32 keeps sub-tick steps.
        world_geometry
            .set_time(((self.world.game_time.rem_euclid(72_000) as f64 + alpha) / 20.0) as f32);
        if let Some((skin, slim)) = vanilla_skin
            && let Some(avatar) = self.player_model.as_mut()
        {
            (avatar.skin, avatar.slim) = (skin, slim);
        }
        // After the mods' world geometry, whose labels add see-through copies.
        let see_through = std::mem::take(&mut self.see_through_vertices);
        let colored_lights = if self.title.is_none() {
            visuals.finish(self, camera)
        } else {
            Vec::new()
        };
        // Map textures and font glyphs changed while building this frame's
        // geometry (game/blocks/maps/mod.rs, world text) upload with this batch; draining
        // map_patches any earlier leaves them a frame behind.
        hud.patches.append(&mut self.map_patches);
        hud.patches.append(self.text_patches.get_mut());
        self.dynamic_texture_commands(&mut hud.dynamic_commands);
        let entity_spans = std::mem::take(self.entity_spans.get_mut());
        let entity_motion = std::mem::take(self.entity_motion.get_mut());
        let entity_motion_posed = std::mem::take(self.entity_motion_posed.get_mut());
        let glowing = std::mem::take(self.glowing_vertices.get_mut());
        let shadow_only = std::mem::take(self.shadow_only_vertices.get_mut());
        let skin = std::mem::take(self.skin_frame.get_mut());
        use sunrise_world::dimension_type::Skybox;
        let skybox = self.dimension_type().skybox;
        let camera_fog = self.camera_fog_type(camera.to_array());
        let boss_world_fog = self.boss_overlay.should_create_world_fog();
        let fast_clouds = self.effective_clouds() == "fast";
        let frame = Frame {
            weather: &weather,
            weather_time: sunrise_client::weather::Time::new(self.world.game_time, alpha as f32),
            nether_lighting: self.nether_lighting(),
            camera,
            yaw,
            pitch,
            roll,
            fov_degrees,
            cull_fov_degrees,
            eye: eye.filter(|_| !title).map_or(camera, |e| e.0),
            eye_rotation: eye.filter(|_| !title).map_or([yaw, pitch], |e| e.1),
            first_person: eye.is_none() || title || !self.view.detached,
            bob,
            view_transform,
            spin,
            sky,
            shadow_scene_revision: (self.world.revision, self.world.chunks.membership_revision()),
            shadow_cache: self.settings.sunrise.shadow_cache,
            distance,
            detail_distance: effective_detail_distance,
            shadow_distance: Some(effective_detail_distance),
            depth_far,
            underwater,
            water_plane: if title
                || underwater
                || !self
                    .renderer
                    .as_ref()
                    .is_some_and(|r| r.wants_water_plane())
            {
                None
            } else {
                self.water_plane(camera)
            },
            title,
            sky_pass: !title
                && sunrise_client::fog::sky_pass_present(camera_fog, sky_effect_blocked, skybox),
            sky_blocked: sunrise_client::fog::sky_pass_skipped(
                camera_fog,
                sky_effect_blocked,
                boss_world_fog,
            ),
            skybox: skybox == Skybox::Overworld && self.settings.sunrise.sky,
            end_sky: skybox == Skybox::End && self.settings.sunrise.sky,
            end_sky_sprite: self
                .gui
                .atlas
                .sprites
                .get("environment/end_sky")
                .map(|r| [r.u0, r.v0, r.u1, r.v1]),
            boss_world_fog,
            environment_fog: (self.dim == Dim::Nether).then_some((sky.fog_start, sky.fog_end)),
            cave_culling,
            water_fog,
            fog: Some(fog),
            dark_disc: self.dark_disc(camera),
            outline,
            outline_after_terrain,
            outline_high_contrast: self.settings.file.bool("highContrastBlockOutline"),
            pvp: self.pvp_render_frame(),
            debug_lines: &debug_lines,
            debug_lines_on_top: &debug_lines_on_top,
            debug_crosshair: &debug_crosshair,
            hud: &hud,
            glint_alpha: self.glint_frame.alpha,
            glint_ticks: self.glint_frame.scroll_ticks(),
            clouds: self
                .frame_cloud_uv(title, &sky)
                .map(|uv| sunrise_client::clouds::Input {
                    texture: &self.pack.clouds,
                    uv,
                    camera,
                    time: self.world.game_time,
                    partial_tick: alpha,
                    range: self.settings.cloud_range,
                    fast: fast_clouds,
                    color: sky.cloud_color,
                    height: sky.cloud_height,
                }),
            entities: &entities,
            explicit_particles: Some(&explicit_particle_quads),
            static_block_entities: &static_block_entities,
            visible_static_block_entity_sections: &visible_static_block_entity_sections,
            static_block_entity_stats,
            entity_motion: &entity_motion,
            entity_motion_posed: &entity_motion_posed,
            entity_spans: &entity_spans,
            skin: skin.pending().then_some(&skin),
            glowing: &glowing,
            dynamic_lights: &self.dynamic_lights,
            colored_lights: &colored_lights,
            emissive_layers: if self.title.is_none() {
                &self.pack.render.emissive_layers
            } else {
                &[]
            },
            lightning: sunrise_client::gfx::lightning::Input {
                bolts: &bolts,
                uv: sunrise_client::gfx::lightning::white_uv(&self.gui.atlas.font.white()),
            },
            additive: &additive,
            see_through: &see_through,
            world_primitives: Default::default(),
            shadow_only: &shadow_only,
            celestial: if self.settings.sunrise.sky || title {
                &celestial
            } else {
                &[]
            },
            // With `celestial` left empty, these have no quad to draw.
            celestial_bodies: &celestial_bodies,
            stars: stars.filter(|_| self.settings.sunrise.sky || title),
            hand: &hand,
            hand_items: &hand_items,
            hand_fov_degrees: hud_fov,
            screen: &screen,
            pack: pack_inputs,
            border,
            game_time: self.world.game_time as f32,
            shader_game_time: ((self.world.game_time % 24000) as f32 + alpha as f32) / 24000.0,
            crumbling_sprite: frame_geometry::crumbling_sprite(
                &self.gui.atlas.sprites,
                self.settings.improved_transparency,
            ),
            end_portal_sprites: ["environment/end_sky", "entity/end_portal/end_portal"].map(|n| {
                self.gui
                    .atlas
                    .sprites
                    .get(n)
                    .map_or([0.0; 4], |r| [r.u0, r.v0, r.u1 - r.u0, r.v1 - r.v0])
            }),
            ticks: self.texture_clock(),
            darkness: self.darkness_lighting(alpha as f32),
            night_vision: self.night_vision_intensity(alpha as f32),
            block_flicker: self.fog.block_light_flicker,
            boss_darken: self
                .boss_overlay
                .darken_amount(self.clock.fraction() as f32),
            menu_blur: self.menu_blur(),
            post_effects: sunrise_client::post_chain::Request {
                effects: &requested_post_effects,
                root: &self.pack.source,
                extras: polytone_extras.as_ref(),
                poly_globals: Some(&post_globals),
                menu_blur_radius: self.settings.menu_blur as f32,
                // GlobalSettingsUniform.update.
                globals: sunrise_client::post_chain::Globals {
                    camera_block_pos: [
                        camera.x.floor() as i32,
                        camera.y.floor() as i32,
                        camera.z.floor() as i32,
                    ],
                    camera_offset: [
                        (camera.x.floor() - camera.x) as f32,
                        (camera.y.floor() - camera.y) as f32,
                        (camera.z.floor() - camera.z) as f32,
                    ],
                    game_time: ((self.world.game_time % 24000) as f32 + alpha as f32) / 24000.0,
                    ..Default::default()
                },
            },
            chunk_fade: self.settings.chunk_fade as f32,
        };
        pr.lap("pr_frame");
        // Renderer encoding and GPU waiting belong to render, not prepare.
        drop(prepare);
        let debug_passes = self.debug_lists_gpu_passes();
        let lightmap_chart = self.debug_shows_lightmap();
        let mut prepared_draws = std::mem::take(&mut *self.prepared_model_draws.borrow_mut());
        let mut renderer_slot = self.renderer.take();
        let result = match renderer_slot.as_mut() {
            Some(renderer) => {
                use sunrise_client::client_perf::request::GpuSwitch;
                let capture_gpu = self.profile.gpu();
                let recording = capture_gpu != GpuSwitch::Off;
                renderer.set_pass_timing(debug_passes || self.gpu_passes || recording);
                renderer.set_lightmap_readback(lightmap_chart);
                renderer.set_keep_timed_frames(recording);
                renderer.set_pass_detail(self.gpu_pass_detail || capture_gpu == GpuSwitch::Detail);
                // `gpu_passes` and a capture's `--gpu` are measurements: they take every sample they can.
                renderer.set_pass_timing_gap(if self.gpu_passes || recording {
                    Duration::ZERO
                } else {
                    sunrise_client::debug_overlay::GPU_TIMING_GAP
                });
                renderer.stage_prepared_draws(&prepared_draws);
                self.render_snapshot.set(Some(RenderSnapshot {
                    framebuffer: [renderer.config.width, renderer.config.height],
                    prepared_generation: renderer.prepared_generation(),
                    staged_prepared_draws: prepared_draws.len() as u64,
                }));
                self.pack_frame_state
                    .set(renderer.shader_pack_state(&frame));
                let mut slots = self.pack_frame_slots.borrow_mut();
                slots.clear();
                slots.extend_from_slice(renderer.pack_bound_mod_slots());
                drop(slots);
                let world_camera = self
                    .native_camera_frame
                    .as_ref()
                    .filter(|_| self.has_world_render_hooks());
                let mut driver = world_camera.map(|camera| {
                    native_custom_geometry::GameWorldStageDriver::new(
                        self,
                        camera,
                        alpha,
                        &mut world_geometry,
                    )
                });
                let rendered = consume(
                    renderer,
                    &frame,
                    driver
                        .as_mut()
                        .map(|d| d as &mut dyn sunrise_client::gfx::world_primitives::FrameHooks),
                );
                self.render_snapshot.set(None);
                rendered
            }
            None => {
                self.pack_frame_state.set(None);
                self.pack_frame_slots.borrow_mut().clear();
                Err("renderer".into())
            }
        };
        drop(frame);
        self.renderer = renderer_slot;
        self.requested_post_effects_buf = requested_post_effects;
        prepared_draws.clear();
        *self.prepared_model_draws.borrow_mut() = prepared_draws;
        let _cleanup = spans::guard("pr_cleanup");
        // A frame the surface refused (occluded, lost, timed out) is `Ok(false)`.
        self.frame_drawn = matches!(result, Ok(true));
        // Restore storage even when rendering fails (for example, a lost surface).
        self.entity_vertices = entities;
        self.explicit_particle_quads = explicit_particle_quads;
        self.world_geometry = Some(world_geometry);
        *self.entity_spans.get_mut() = entity_spans;
        *self.entity_motion.get_mut() = entity_motion;
        *self.entity_motion_posed.get_mut() = entity_motion_posed;
        *self.glowing_vertices.get_mut() = glowing;
        *self.shadow_only_vertices.get_mut() = shadow_only;
        let mut skin = skin;
        // The renderer took the new models when its store grew by them; otherwise they stay
        // pending, or the stores restart (`SkinModels::begin`).
        if self
            .renderer
            .as_ref()
            .and_then(|r| r.skinned_models())
            .is_some_and(|stored| stored == skin.base + skin.new_model.len() as u32)
        {
            skin.handed();
        }
        *self.skin_frame.get_mut() = skin;
        self.see_through_vertices = see_through;
        result
    }
}
/// Read-only renderer state cached while `Game::renderer` is temporarily taken for
/// `Renderer::render` so `ClientHost` queries keep working inside stage callbacks.
#[derive(Clone, Copy)]
pub(super) struct RenderSnapshot {
    pub(super) framebuffer: [u32; 2],
    pub(super) prepared_generation: Option<u64>,
    pub(super) staged_prepared_draws: u64,
}

impl Game<'_> {
    pub(super) fn has_renderer(&self) -> bool {
        self.renderer.is_some() || self.render_snapshot.get().is_some()
    }
    pub(super) fn renderer_framebuffer(&self) -> Option<[u32; 2]> {
        self.renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
            .or_else(|| self.render_snapshot.get().map(|s| s.framebuffer))
    }
    pub(super) fn renderer_prepared_generation(&self) -> Option<u64> {
        self.renderer.as_ref().map_or_else(
            || {
                self.render_snapshot
                    .get()
                    .and_then(|s| s.prepared_generation)
            },
            |r| r.prepared_generation(),
        )
    }
    pub(super) fn renderer_prepared_room_info(&self) -> Option<(u64, u64)> {
        let max_buffer_size = self.mod_gpu.as_ref()?.max_buffer_size();
        let staged = match self.renderer {
            Some(_) => 0,
            None => self
                .render_snapshot
                .get()
                .map_or(0, |s| s.staged_prepared_draws),
        };
        Some((max_buffer_size, staged))
    }
    /// The mod GPU handles: always there when a renderer is, also during `Renderer::render`.
    pub(in crate::game) fn mod_gpu(&self) -> Option<&sunrise_client::gfx::ModGpuResources> {
        self.mod_gpu.as_ref()
    }
    /// Drops the mod GPU handles and the renderer together.
    pub(in crate::game) fn clear_renderer(&mut self) {
        self.mod_gpu = None;
        self.renderer = None;
    }
    /// Installs `renderer` and the handles mods reach it through.
    pub(in crate::game) fn set_renderer(&mut self, renderer: Renderer) {
        self.mod_gpu = Some(renderer.mod_gpu().clone());
        self.renderer = Some(renderer);
    }
}

/// `computeNextAutosaveInterval`'s arithmetic: `Math.max(100, (int)(rate * 300.0F))`.
fn autosave_interval(rate: f32) -> i64 {
    i64::from((rate * 300.0) as i32).max(100)
}

/// The parts of a window key used by IME and preset editor dispatch. The original winit
/// event carries platform data for the rest of the window's input handling.
struct WindowKeyInput<'a> {
    physical: PhysicalKey,
    state: ElementState,
    repeat: bool,
    text: Option<&'a str>,
    shortcut: Option<sunrise_client::clipboard::EditShortcut>,
}

/// winit's events for a window's game, with any loop: a headless client (#4951) has no
/// winit loop and calls these from its own.
impl Game<'_> {
    fn pointer_space(&self, framebuffer: [u32; 2]) -> sunrise_client::window::PointerSpace {
        use sunrise_client::window::PointerSpace;
        match self.window.as_deref() {
            Some(window) => *self
                .window_pointer_space
                .get_or_init(|| PointerSpace::from_window(window)),
            _ => PointerSpace::new(framebuffer, 1.0),
        }
    }

    fn pointer_projection(
        &self,
        framebuffer: [u32; 2],
    ) -> sunrise_client::window::PointerProjection {
        use sunrise_client::window::{GuiSize, PointerProjection};
        let inputs = self.settings.gui_scale_inputs();
        PointerProjection {
            gui: GuiSize::new(framebuffer, inputs.requested, inputs.unicode),
            window: self.pointer_space(framebuffer),
        }
    }

    fn renderer_pointer_projection(&self) -> Option<sunrise_client::window::PointerProjection> {
        let r = self.renderer.as_ref()?;
        Some(self.pointer_projection([r.config.width, r.config.height]))
    }

    fn project_window_cursor(
        &self,
        framebuffer: [u32; 2],
    ) -> Option<sunrise_client::window::GuiPointer> {
        self.window_cursor.map(|position| {
            self.pointer_projection(framebuffer)
                .project_pointer(position)
        })
    }

    fn set_window_cursor(
        &mut self,
        sample: sunrise_client::window::PointerSample,
        projection: sunrise_client::window::PointerProjection,
    ) {
        self.window_cursor = Some(sample);
        self.cursor = projection.project_pointer(sample).subpixel();
    }

    fn refresh_window_cursor(&mut self) {
        if let Some(position) = self.window_cursor
            && let Some(projection) = self.renderer_pointer_projection()
        {
            self.cursor = projection.project_pointer(position).subpixel();
        }
    }

    /// Dispatch a window key through IME, the window's shortcuts and overlays, then any bound
    /// preset editor. `true` means no further screen or gameplay handling is needed.
    fn dispatch_window_key(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        input: WindowKeyInput<'_>,
        before_screen: impl FnOnce(&mut Self) -> bool,
    ) -> bool {
        let pressed = input.state == ElementState::Pressed;
        if pressed && !input.repeat && !self.pad.menu.synthetic {
            self.pad_radial_cancel();
        }
        if let PhysicalKey::Code(key) = input.physical {
            if pressed {
                // Only a press that finds no field focused can open one.
                let text = input
                    .text
                    .filter(|_| self.preset_editor_title().is_none() && !self.text_field_focused());
                self.held_key_text.pressed(key, text);
            } else {
                self.held_key_text.released(key);
            }
        }
        // The OS consumes composing keys before they can reach a screen or a shortcut.
        if (pressed && self.ime_suppresses_key()) || before_screen(self) {
            return true;
        }
        let key = match input.physical {
            PhysicalKey::Code(key) => Some(key),
            PhysicalKey::Unidentified(_) => None,
        };
        if let Err(error) = self.dispatch_window_keyboard(
            event_loop,
            key,
            pressed,
            input.repeat,
            input.shortcut,
            input.text,
        ) {
            self.error = Some(error);
            event_loop.exit();
        }
        true
    }

    /// Window bookkeeping, shortcuts and overlays before dispatching to the screen. Returns
    /// `true` when an early handler consumed the event.
    fn prepare_window_event(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        event: &WindowEvent,
        cursor_sample: Option<sunrise_client::window::PointerSample>,
    ) -> bool {
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
        ) {
            self.window_pointer_space.take();
        }
        if matches!(
            event,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            }
        ) && !self.pad.menu.synthetic
        {
            self.pad_radial_cancel();
        }
        if let WindowEvent::Focused(false) = event {
            self.modifiers = winit::keyboard::ModifiersState::empty();
            self.physical_keys.clear();
            self.physical_buttons.clear();
            self.drag_button = None;
            self.container_buttons.clear();
            self.mod_browser_focus_lost();
        }
        if let WindowEvent::ModifiersChanged(m) = event {
            self.update_modifiers(m.state());
        }
        if let WindowEvent::Focused(false) = event {
            self.update_modifiers(winit::keyboard::ModifiersState::empty());
            self.debug_keys.reset();
        }
        // Pack drops on the resource-pack and Create New World screens.
        if let WindowEvent::DroppedFile(path) = event {
            self.drop_pack_file(path);
            if mod_api::__private::listened(mod_api::HookId::ScreenFilesDropped) {
                self.dropped_files.push(path.clone());
            }
        }
        // Fullscreen is a window shortcut, before screens consume keyboard input.
        // Not Alt or Super on their own: leaving the window isn't playing with the keyboard.
        if let WindowEvent::KeyboardInput { event, .. } = event
            && event.state == ElementState::Pressed
            && !event.repeat
            && !matches!(
                event.physical_key,
                PhysicalKey::Code(
                    KeyCode::AltLeft | KeyCode::AltRight | KeyCode::SuperLeft | KeyCode::SuperRight
                )
            )
        {
            self.pad.keyboard_took_over = true;
        }
        if let WindowEvent::KeyboardInput { event, .. } = event
            && event.state == ElementState::Pressed
            && let PhysicalKey::Code(key) = event.physical_key
            && sunrise_client::keybinds::key_name(key)
                == Some(self.settings.keys.get("key.fullscreen"))
            && !(self.screen == Screen::Options && self.options_waiting_for_key())
        {
            self.last_input = Instant::now();
            if !event.repeat {
                self.toggle_fullscreen();
            }
            return true;
        }
        // A screenshot is a window shortcut too: vanilla takes it before a
        // screen or a text box consumes the key (`handleGlobalKeyPress`), so
        // it works with chat, a sign or a book open.
        if let WindowEvent::KeyboardInput { event, .. } = event
            && event.state == ElementState::Pressed
            && let PhysicalKey::Code(key) = event.physical_key
        {
            self.last_input = Instant::now();
            if self.screenshot_shortcut(key, event.repeat) {
                return true;
            }
        }
        // KeyboardHandler.keyPress: the debug keys come before the game's.
        if let WindowEvent::KeyboardInput {
            event: key_event, ..
        } = event
            && let PhysicalKey::Code(key) = key_event.physical_key
        {
            use sunrise_client::debug_keys::KeyAction;
            let action = match (key_event.state, key_event.repeat) {
                (ElementState::Released, _) => KeyAction::Release,
                (_, false) => KeyAction::Press,
                (_, true) => KeyAction::Repeat,
            };
            if self.debug_key(key, action) {
                return true;
            }
        }
        // KeyboardHandler.keyPress: the narrator hotkey, on every screen and
        // in game, before the screen takes the key.
        if let WindowEvent::KeyboardInput { event, .. } = event {
            self.narrator_hotkey(event);
        }
        if matches!(
            event,
            WindowEvent::KeyboardInput { .. }
                | WindowEvent::MouseInput { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::CursorMoved { .. }
        ) {
            self.last_input = Instant::now();
        }
        if matches!(
            event,
            WindowEvent::MouseInput { .. } | WindowEvent::MouseWheel { .. }
        ) {
            if !self.pad_synthetic_input() {
                self.reassert_pointer_capture();
            }
            // A click can arrive before the first redraw at a changed GUI size.
            self.refresh_window_cursor();
        }
        // The real mouse takes the pointer back from the keyboard's focus.
        self.key_focus_mouse(event_loop, event, cursor_sample);
        // The screen's narration hears of the input before the screen takes
        // it; the pointer events the keyboard's focus sends aren't the mouse.
        if !self.key_focus_quiet() {
            narrator::narrate_input(&mut self.narration, event, self.cursor);
        }
        // FileFixerProgressScreen: Cancel is the only way out (`shouldCloseOnEsc`
        // is false), so it takes the pointer and Escape does nothing.
        if let Some(starting) = &self.integrated_starting
            && starting.handle.upgrade().is_file_fixing()
        {
            let upgrade = starting.handle.upgrade();
            match event {
                WindowEvent::CursorMoved { .. } => {
                    if let (Some(projection), Some(sample)) =
                        (self.renderer_pointer_projection(), cursor_sample)
                    {
                        self.set_window_cursor(sample, projection);
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } if !upgrade.is_canceled() => {
                    let (w, h) = self.gui_size();
                    let (x, y) = sunrise_client::level_loading::file_fixer_cancel(w, h);
                    if sunrise_client::ui::hovered(self.cursor, x, y, 200.0, 20.0) {
                        upgrade.set_canceled();
                        self.play_audio_button_click();
                    }
                }
                _ => {}
            }
            if matches!(event, WindowEvent::KeyboardInput { .. }) {
                return true;
            }
        }
        if self.pending_entry.is_some()
            && matches!(event, WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.physical_key == PhysicalKey::Code(KeyCode::Escape))
        {
            self.pending_entry = None;
            self.perf.cancel_world_opening();
            return true;
        }
        if (self.pending_entry.is_some() || self.integrated_starting.is_some())
            && !matches!(
                event,
                WindowEvent::CloseRequested
                    | WindowEvent::Resized(_)
                    | WindowEvent::RedrawRequested
            )
        {
            return true;
        }
        // The pack reload's overlay takes no input until it has faded out
        // (`LoadingOverlay`): `spawn_pack_reload` released what was held.
        if self.reload_overlay.is_some()
            && !matches!(
                event,
                WindowEvent::CloseRequested
                    | WindowEvent::Focused(_)
                    | WindowEvent::Resized(_)
                    | WindowEvent::RedrawRequested
            )
        {
            return true;
        }
        if self.pending_save.is_some() {
            match event {
                WindowEvent::CloseRequested => {
                    if (self.settings.quit_shortcuts
                        || !sunrise_client::input::is_quit_shortcut_down(
                            self.modifiers,
                            &self.physical_keys,
                        ))
                        && let Err(error) = self.begin_closing_save(true)
                    {
                        self.error = Some(error);
                        event_loop.exit();
                    }
                }
                WindowEvent::Focused(focused) => {
                    self.focused = *focused;
                    if !*focused {
                        self.capture(false);
                    }
                }
                WindowEvent::Resized(size) => {
                    if let Some(r) = &mut self.renderer {
                        r.resize(size.width, size.height);
                    }
                }
                WindowEvent::RedrawRequested => {
                    if let Err(error) = self.render_closing_save() {
                        self.error = Some(error);
                        event_loop.exit();
                    }
                }
                _ => {}
            }
            return true;
        }
        false
    }

    /// Dispatch the button to the top screen, not to the gameplay mappings underneath it.
    fn route_screen_button(
        &mut self,
        button: MouseButton,
        code: i32,
        pressed: bool,
        double_click: bool,
    ) -> Result<bool> {
        let left = button == MouseButton::Left;
        let left_press = pressed && left;
        if self.command_prompt.is_some() {
            return Ok(left_press && self.command_prompt_click());
        }
        if self.link_prompt.is_some() {
            return Ok(left_press && self.link_prompt_click());
        }
        if self.code_of_conduct.is_some() {
            return Ok(left_press && self.code_of_conduct_click()?);
        }
        if self.server_packs.prompt.is_some() {
            return Ok(left_press && self.pack_prompt_click()?);
        }
        if self.dialog_open() {
            return if !left {
                Ok(false)
            } else if pressed {
                self.dialog_click()
            } else {
                Ok(self.dialog_release())
            };
        }
        // ConnectScreen's Cancel, under the prompts and dialogs above.
        if self.remote_loading() {
            return Ok(left_press && self.connect_screen_click()?);
        }
        if self.screen == Screen::Win {
            return Ok(false);
        }
        if self.screen == Screen::Options {
            return if self.options_waiting_for_key() && pressed {
                self.options_bind_mouse(button)?;
                Ok(true)
            } else if matches!(button, MouseButton::Left | MouseButton::Right) {
                self.options_mouse(pressed, !left)
            } else {
                Ok(false)
            };
        }
        if self.title_input_active() {
            if !pressed {
                return Ok(left
                    && self.title.as_mut().is_some_and(|title| {
                        let released = title.release();
                        title.release_field() || released
                    }));
            }
            if !matches!(button, MouseButton::Left | MouseButton::Middle) {
                return Ok(self.preset_editor_mouse_clicked(code, double_click));
            }
            let size = self
                .renderer
                .as_ref()
                .map_or([1, 1], |r| [r.config.width, r.config.height]);
            let b = HudBatch::new(size, self.settings.gui_scale_inputs());
            if button == MouseButton::Middle {
                // The world list takes any button; middle selects without joining.
                return Ok(self.preset_editor_mouse_clicked(code, double_click)
                    || self.title.as_mut().is_some_and(|title| {
                        title.select_world_at(self.cursor.0, self.cursor.1, (b.width, b.height))
                    }));
            }
            let atlas = &self.gui.atlas;
            let shift = self.has_shift_down();
            let (action, sound, widget) =
                self.title
                    .as_mut()
                    .map_or((Action::None, false, false), |title| {
                        let (action, sound, widget) = title.click_with_outcome(
                            self.cursor.0,
                            self.cursor.1,
                            (b.width, b.height),
                        );
                        let field =
                            title.press_field(atlas, self.cursor, (b.width, b.height), shift);
                        let (rule_sound, rules) =
                            title.press_rules_with_outcome(atlas, self.cursor, (b.width, b.height));
                        (action, sound || rule_sound, widget || field || rules)
                    });
            // Done/Cancel are the bound editor's normal widgets. They take the click even
            // when the actual mode change is deferred to `settle_title_mode`.
            let handled = widget || self.preset_editor_mouse_clicked(code, double_click);
            self.save_title_options();
            if sound {
                self.play_audio_button_click();
            }
            self.title_action(action)?;
            self.reopen_data_packs()?;
            return Ok(handled);
        }
        if self.book.is_some() || self.sign_edit.is_some() {
            if !left_press {
                return Ok(false);
            }
            let size = self
                .renderer
                .as_ref()
                .map_or([1, 1], |r| [r.config.width, r.config.height]);
            return Ok(if self.book.is_some() {
                self.book_click(size)
            } else {
                self.sign_edit_click(size)
            });
        }
        if self.studio_mouse_input(pressed, button) {
            return Ok(true);
        }
        if matches!(self.screen, Screen::Inventory | Screen::Creative)
            && let Some(key) = self
                .settings
                .keys
                .container_mouse_key(button, self.creative)
        {
            if pressed && self.carried.is_none() {
                self.inventory_hotkey(key);
            }
            // Neither edge of a bound button is an ordinary slot click.
            return Ok(true);
        }
        if pressed
            && (self.sort_mouse_press(button)
                || self.hud_editor_key_press(&sunrise_client::keybinds::mouse_name(button)))
        {
            return Ok(true);
        }
        // AbstractContainerScreen.mouseClicked (also Creative's fallback) handles
        // every press, even when no widget or slot accepts it. Releases answer separately.
        Ok(match self.screen {
            Screen::Inventory | Screen::Creative
                if !matches!(button, MouseButton::Left | MouseButton::Right) =>
            {
                if self.settings.keys.get("key.pickItem")
                    != sunrise_client::keybinds::mouse_name(button)
                {
                    return Ok(pressed);
                }
                let handled = if pressed {
                    self.inventory_pick_press();
                    true
                } else {
                    self.inventory_pick_release()
                };
                if let Some(key) = self.container {
                    self.container_changed(key);
                }
                handled
            }
            Screen::Inventory if pressed => {
                self.inventory_press(!left);
                if let Some(key) = self.container {
                    self.container_changed(key);
                }
                true
            }
            Screen::Inventory => {
                self.inventory_mouse_release(!left)
                    || (self.station_release() | self.recipe_book_release())
            }
            Screen::Chat if left_press => self.chat_click(),
            Screen::Creative if pressed => {
                self.click_creative(!left);
                true
            }
            Screen::Creative => {
                self.inventory_mouse_release(!left) || (left && self.creative_screen.release())
            }
            Screen::Paused if left_press && self.pause_menu => self.pause_click()?,
            Screen::WorldOptions if left_press => self.world_options_click(),
            Screen::ServerLinks if left_press => self.server_links_click()?,
            Screen::DebugOptions if left_press => self.debug_options_click(),
            Screen::DebugOptions if left => self.debug_options_release(),
            Screen::HudEditor
                if pressed && matches!(button, MouseButton::Left | MouseButton::Right) =>
            {
                self.hud_editor_press(!left)
            }
            Screen::HudEditor if left => self.hud_editor_release(),
            Screen::ConfirmLink if left_press => self.confirm_link_click(),
            Screen::DemoIntro if left_press => self.demo_intro_click(),
            Screen::DialogWarning if left_press => self.dialog_warning_click()?,
            Screen::WorldOptions if left => self.world_options_release(),
            Screen::Friends if left_press => self.friends_screen_click(),
            Screen::Social if left_press => self.social_click(),
            Screen::CommandBlock if left_press => self.command_block_click(),
            Screen::BlockEditor if left_press => self.block_editor_click(),
            Screen::BlockEditor if left => self.block_editor_release(),
            Screen::Dead if left_press => self.death_click()?,
            Screen::Advancements if left => {
                let size = self
                    .renderer
                    .as_ref()
                    .map_or([1, 1], |r| [r.config.width, r.config.height]);
                self.advancements_mouse(pressed, size)
            }
            Screen::Stats if left_press => {
                let size = self
                    .renderer
                    .as_ref()
                    .map_or([1, 1], |r| [r.config.width, r.config.height]);
                self.stats_click(size)
            }
            Screen::Stats if left => self.stats_release(),
            Screen::GameModeSwitcher if !pressed => self.switcher_release(button),
            _ => false,
        })
    }

    /// One window event, in the order vanilla's `KeyboardHandler` and `MouseHandler` see it.
    ///
    /// A controller doesn't come through here in play: `pad_action` goes straight to the mappings
    /// (`mapped_key`, `mouse_mapping`). So a key handled below before `mapped_key_event` (a window
    /// shortcut, a special case) never reaches a controller. If a player could want it on one, give
    /// it a handler both can call and a `gamepad::Route` (screenshot and fullscreen do this). A
    /// screen's own raw keys, text and mods' key hooks stay raw on purpose.
    /// `mouse_press_millis` is the monotonic timestamp of a physical press, absent for synthetic
    /// inputs and all other events.
    /// Returns true when button routing already settled the title before its Post.
    fn handle_window_event_inner(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        event: WindowEvent,
        cursor_sample: Option<sunrise_client::window::PointerSample>,
        mouse_press_millis: Option<u64>,
    ) -> bool {
        let before_screen =
            |game: &mut Self| game.prepare_window_event(event_loop, &event, cursor_sample);
        let handled = match &event {
            WindowEvent::KeyboardInput { event: key, .. } => self.dispatch_window_key(
                event_loop,
                WindowKeyInput {
                    physical: key.physical_key,
                    state: key.state,
                    repeat: key.repeat,
                    text: key.text.as_deref(),
                    shortcut: self.edit_shortcut(key),
                },
                before_screen,
            ),
            _ => before_screen(self),
        };
        if handled {
            return false;
        }
        let initial_instance = self.screen_state.screen_instance;
        let mut mouse_post: Option<native_hooks::ScreenMousePost> = None;
        let mut title_settled = false;
        let result: Result<()> = (|| {
            match event {
                WindowEvent::CloseRequested => {
                    if !self.settings.quit_shortcuts
                        && sunrise_client::input::is_quit_shortcut_down(
                            self.modifiers,
                            &self.physical_keys,
                        )
                    {
                        return Ok(());
                    }
                    self.agent_cancel("the game window was closed; the game saves and exits");
                    self.begin_closing_save(true)?;
                }
                // Input method composition: the committed text types like a key's text.
                WindowEvent::Ime(winit::event::Ime::Commit(text)) => self.ime_commit(&text),
                WindowEvent::Ime(winit::event::Ime::Preedit(text, cursor)) => {
                    self.ime_preedit(&text, cursor)
                }
                WindowEvent::Ime(winit::event::Ime::Disabled) => {
                    sunrise_client::preedit::set_preedit(None)
                }
                WindowEvent::Resized(size) => {
                    if let Some(r) = &mut self.renderer {
                        r.resize(size.width, size.height);
                    }
                    self.refresh_window_cursor();
                    self.observe_fullscreen();
                }
                WindowEvent::Focused(focused) => {
                    self.focused = focused;
                    self.focus_lost = (!focused).then(Instant::now);
                    if focused {
                        self.regrab_on_refocus();
                    } else {
                        // A held pointer goes with the focus, wherever the
                        // system keeps the grab: the window asks for it back
                        // when the focus returns (`regrab_on_refocus`).
                        let held = self.captured;
                        if self.control.is_none() {
                            self.keys.clear();
                            let options = self.toggle_options();
                            self.vanilla_mappings.set_keys_up_on_focus_loss(options);
                            // GLFW releases every held key and button when
                            // the window loses focus, so a mod's mappings go
                            // up as vanilla's do; their pending clicks stay.
                            self.mod_keys.set_all_up();
                            self.capture(false);
                        } else if self.screen_grab {
                            // A mod screen's grab goes with the focus under a control
                            // connection too; the game's own grab stays as it was.
                            self.capture(false);
                        }
                        self.regrab_on_refocus = held;
                    }
                }
                WindowEvent::CursorMoved { .. } => {
                    // A screen holding the pointer grab hears only `screen_pointer_motion`:
                    // where the system merely confines the hidden cursor (Windows, X11), its
                    // moves leave the screen's cursor and drags alone.
                    if self.mod_screen_grabs_pointer() {
                        return Ok(());
                    }
                    if let (Some(r), Some(sample)) = (&self.renderer, cursor_sample) {
                        let size = [r.config.width, r.config.height];
                        let projection = self.pointer_projection(size);
                        let delta = self
                            .window_cursor
                            .map_or([0.0; 2], |previous| projection.drag_delta(previous, sample));
                        self.set_window_cursor(sample, projection);
                        let [w, h] = projection.gui.size.map(|v| v as f32);
                        let overlay = self.observed_overlay_now();
                        if self.observed_screen_view().is_none() && self.studio_mouse_dragged(w) {
                            return Ok(());
                        }
                        let button = if self.pad_synthetic_input() {
                            if self.pad.menu.held[0] {
                                Some(MouseButton::Left)
                            } else {
                                self.pad.menu.held[1].then_some(MouseButton::Right)
                            }
                        } else {
                            self.drag_button
                                .filter(|b| self.physical_buttons.contains(b))
                        };
                        if self.screen_mouse_moved(delta) {
                            return Ok(());
                        }
                        if self.screen_state.screen_instance != initial_instance {
                            return Ok(());
                        }
                        match self.screen_mouse_dragged(delta) {
                            native_hooks::ScreenMousePre::Canceled => return Ok(()),
                            native_hooks::ScreenMousePre::Passed(post) => mouse_post = Some(post),
                            native_hooks::ScreenMousePre::NotFired => {}
                        }
                        if self.screen_state.screen_instance != initial_instance {
                            return Ok(());
                        }
                        if delta != [0.0; 2]
                            && let Some(button) = button
                        {
                            // A press on the item panel's list is the panel's until its
                            // release: no widget, menu or slot hears the drag.
                            let panel = self.mod_browser_drag_move();
                            let handled = panel
                                || (self.observed_screen_view().is_some()
                                    && self.mounted_widget_mouse_dragged(
                                        self.cursor(),
                                        glfw_button(button),
                                        delta,
                                        glfw_modifiers(self.modifiers),
                                    ));
                            let handled = handled
                                || (self.screen_state.screen_instance == initial_instance
                                    && self.screen_drag_response(overlay, button, size, (w, h))?);
                            if let Some(post) = mouse_post.as_mut() {
                                post.handled = handled;
                            }
                        }
                    }
                }
                WindowEvent::MouseWheel { .. }
                    if self.screen == Screen::Playing && self.replay.driver.is_some() => {}
                WindowEvent::MouseWheel { delta, .. } => {
                    let (raw_x, raw_y) = match delta {
                        MouseScrollDelta::LineDelta(x, y) => (f64::from(x), f64::from(y)),
                        MouseScrollDelta::PixelDelta(p) => (p.x / 30.0, p.y / 30.0),
                    };
                    // MouseHandler.onScroll: discrete scrolling and the wheel sensitivity.
                    let discrete = self.settings.discrete_scroll;
                    let wheel_sensitivity = self.settings.mouse_wheel_sensitivity;
                    let scaled = |v: f64| {
                        (if discrete {
                            sunrise_client::mouse::java_signum(v)
                        } else {
                            v
                        }) * wheel_sensitivity
                    };
                    let d = scaled(raw_y) as f32;
                    // ScreenEvent.MouseScrolled.Pre: the open screen hears of
                    // the wheel before anything scrolls, and a cancelled turn
                    // scrolls nothing — no bundle, no list, slider or
                    // scrollbar. The hotbar below reads the wheel only while
                    // playing, which the gate excludes. A wheel the pad or the
                    // keyboard's focus made up is not the mouse: it fires
                    // nothing.
                    let scroll = [scaled(raw_x), scaled(raw_y)];
                    if !self.pad_synthetic_input() {
                        let modifiers = glfw_modifiers(self.modifiers);
                        if let Some(receiver) = self.observed_input_screen() {
                            let mut canceled = false;
                            self.native_screen_mouse_scrolled_pre(
                                &receiver.view(),
                                scroll,
                                modifiers,
                                &mut canceled,
                            );
                            if canceled {
                                return Ok(());
                            }
                            mouse_post = Some(native_hooks::ScreenMousePost::new(
                                receiver,
                                modifiers,
                                native_hooks::ScreenMousePostKind::Scroll(scroll),
                            ));
                            if self.screen_state.screen_instance != initial_instance {
                                return Ok(());
                            }
                        }
                        if self.observed_screen_view().is_some() {
                            if self.mounted_widget_mouse_scrolled(self.cursor(), scroll, modifiers)
                            {
                                if let Some(post) = mouse_post.as_mut() {
                                    post.handled = true;
                                }
                                return Ok(());
                            }
                            if self.screen_state.screen_instance != initial_instance {
                                return Ok(());
                            }
                        }
                        if mouse_post.is_some() && self.container_wheel(d) {
                            if let Some(post) = mouse_post.as_mut() {
                                post.handled = true;
                            }
                            return Ok(());
                        }
                        if self.screen_state.screen_instance != initial_instance {
                            return Ok(());
                        }
                    }
                    // Only the topmost overlay is the receiving screen.
                    if let Some(overlay) = self.observed_overlay_now() {
                        let handled = match overlay {
                            "code_of_conduct" => self.code_of_conduct_scroll(d),
                            "dialog" => self.dialog_scroll(d),
                            _ => false,
                        };
                        if let Some(post) = mouse_post.as_mut() {
                            post.handled = handled;
                        }
                        return Ok(());
                    }
                    // The item panel beside a container screen pages on the
                    // wheel over it, as JEI's cancelled MouseScrolled.Pre: the
                    // screen underneath (bundle, creative tabs' list, station)
                    // and the Post event hear nothing of that notch.
                    if d != 0.0 && self.mod_browser_wheel(d) {
                        if let Some(post) = mouse_post.as_mut() {
                            post.handled = true;
                        }
                        return Ok(());
                    }
                    let mut screen_scrolled = false;
                    let stick_scroll = self.pad_stick_scroll();
                    let step_scroll = self.pad_step_scroll();
                    if self.screen == Screen::Playing
                        && let Some(title) = &mut self.title
                    {
                        let size = self
                            .window
                            .as_ref()
                            .map(|w| w.inner_size())
                            .unwrap_or_default();
                        let batch = HudBatch::new(
                            [size.width, size.height],
                            self.settings.gui_scale_inputs(),
                        );
                        let gui = (batch.width, batch.height);
                        if step_scroll {
                            title.step_scroll(d, self.cursor, gui);
                        } else if stick_scroll {
                            title.stick_scroll(d, self.cursor, gui);
                        } else {
                            screen_scrolled = title.scroll_worlds(d, self.cursor, gui);
                        }
                    }
                    // MouseHandler.onScroll: the hotbar (or the spectator
                    // menu) takes the wheel only when no screen is open; a
                    // book, lectern, sign or prompt drawn over gameplay is one.
                    if self.hotbar_takes_wheel() {
                        let scroll =
                            self.mouse
                                .on_scroll(raw_x, raw_y, discrete, wheel_sensitivity);
                        let (horizontal_steps, vertical_steps) =
                            scroll.map_or((0, 0), |(_, steps)| steps);
                        if self.hotbar_scroll_canceled(
                            [scaled(raw_x), scaled(raw_y)],
                            [horizontal_steps, vertical_steps],
                        ) {
                            return Ok(());
                        }
                        let Some((wheel, _)) = scroll else {
                            return Ok(());
                        };
                        self.hotbar_wheel_step(wheel, vertical_steps);
                    }
                    match self.screen {
                        // ChatScreen.mouseScrolled always takes the wheel.
                        Screen::Chat => {
                            self.chat_scroll(d);
                            screen_scrolled = true;
                        }
                        Screen::Social => screen_scrolled |= self.social_scroll(d),
                        Screen::CommandBlock => screen_scrolled |= self.command_block_scroll(d),
                        Screen::Options => screen_scrolled |= self.options_scroll(d)?,
                        _ => {}
                    }
                    // AbstractContainerScreen.mouseScrolled: a bundle under the
                    // cursor takes the wheel before the screen's own scrolling.
                    let bundle = bundles::takes_bundle_wheel(self.screen)
                        && d != 0.0
                        && self.bundle_scroll(scaled(raw_x), scaled(raw_y));
                    if bundle {
                        screen_scrolled = true;
                    }
                    if self.screen == Screen::Creative
                        && d != 0.0
                        && !bundle
                        && self.creative_screen.can_scroll(&self.pack.names)
                    {
                        self.creative_screen.wheel(f64::from(d), &self.pack.names);
                        screen_scrolled = true;
                    }
                    if self.screen == Screen::Inventory
                        && d != 0.0
                        && !bundle
                        && (self.tweak_scroll(d, self.inventory_target().0)
                            || self.station_scroll(d))
                    {
                        screen_scrolled = true;
                    }
                    screen_scrolled |= match self.screen {
                        Screen::Stats if d != 0.0 => self.stats_scroll(d),
                        Screen::WorldOptions if d != 0.0 => self.world_options_scroll(d),
                        Screen::ServerLinks if d != 0.0 => self.server_links_scroll(d),
                        Screen::DebugOptions if d != 0.0 => self.debug_options_scroll(d),
                        Screen::HudEditor if d != 0.0 => self.hud_editor_scroll(f64::from(d)),
                        Screen::Advancements => self.advancements_scroll(scroll[0] as f32, d),
                        _ => false,
                    };
                    if let Some(post) = mouse_post.as_mut() {
                        post.handled = screen_scrolled;
                    }
                }
                WindowEvent::MouseInput { state, button, .. } => {
                    let pressed = state == ElementState::Pressed;
                    let code = glfw_button(button);
                    let modifiers = glfw_modifiers(self.modifiers);
                    let synthetic = self.pad_synthetic_input();
                    let owned_release = std::mem::take(&mut self.container_owned_up);
                    let had_screen = self.mod_mappings_screen_open() || self.remote_loading();
                    let receiver = (!synthetic).then(|| self.observed_input_screen()).flatten();
                    let title_mode = (self.screen == Screen::Playing
                        && !self.screen_over_playing())
                    .then(|| {
                        self.title
                            .as_ref()
                            .map(sunrise_client::title::Title::mode_activation)
                    })
                    .flatten();
                    let press_millis = mouse_press_millis.filter(|_| receiver.is_some());
                    let double_click = press_millis.is_some_and(|millis| {
                        self.screen_state.is_double_click(
                            millis,
                            initial_instance,
                            title_mode,
                            code,
                        )
                    });
                    if !synthetic {
                        self.track_drag_button(button, pressed);
                    }
                    if pressed {
                        self.mod_browser_other_press(button == MouseButton::Left);
                    }
                    if let Some(receiver) = &receiver {
                        let mut canceled = false;
                        if pressed {
                            self.native_screen_mouse_button_pressed_pre(
                                &receiver.view(),
                                code,
                                modifiers,
                                double_click,
                                &mut canceled,
                            );
                        } else {
                            self.native_screen_mouse_button_released_pre(
                                &receiver.view(),
                                code,
                                modifiers,
                                &mut canceled,
                            );
                        }
                        if canceled {
                            // A vetoed release of a press on the item panel
                            // drops its item, sending nothing.
                            if !pressed && button == MouseButton::Left {
                                self.mod_browser_release_vetoed();
                            }
                            if self.screen_state.screen_instance == initial_instance
                                && let Some(millis) = press_millis
                            {
                                self.screen_state.record_click(
                                    millis,
                                    initial_instance,
                                    title_mode,
                                    code,
                                );
                            }
                            return Ok(());
                        }
                    }
                    // The left release of a press on the item panel's list is
                    // the panel's, even once its menu closed or was replaced:
                    // no screen, mounted widget or container hook hears it.
                    let panel_release =
                        !pressed && button == MouseButton::Left && self.mod_browser_release();
                    let mut handled = owned_release || panel_release;
                    if !panel_release && self.screen_state.screen_instance == initial_instance {
                        let inventory_release = !pressed
                            && matches!(self.screen, Screen::Inventory | Screen::Creative)
                            && self.observed_overlay_now().is_none()
                            && !self.remote_loading();
                        let native_child = pressed.then(|| self.native_pointer_child()).flatten();
                        if let Some(child) = native_child {
                            // A hit belongs to the first ordered child even if it declines.
                            // Only the child's own answer can change focus.
                            let accepted =
                                self.route_screen_button(button, code, true, double_click)?;
                            if accepted
                                && button == MouseButton::Left
                                && self.screen_state.screen_instance == initial_instance
                            {
                                self.focus_native_pointer_child(child);
                            }
                            handled = true;
                        } else {
                            if !owned_release
                                && !inventory_release
                                && !self.key_focus_driving()
                                && self.observed_screen_view().is_some()
                            {
                                let mouse = self.cursor();
                                handled = if pressed {
                                    self.mounted_widget_mouse_clicked(
                                        mouse,
                                        code,
                                        modifiers,
                                        double_click,
                                    )
                                } else {
                                    self.mounted_widget_mouse_released(mouse, code, modifiers)
                                };
                            }
                            if !handled && self.screen_state.screen_instance == initial_instance {
                                handled = receiver.is_some()
                                    && self.container_mouse(button, pressed, false);
                            }
                            if !handled && self.screen_state.screen_instance == initial_instance {
                                handled =
                                    self.route_screen_button(button, code, pressed, double_click)?;
                            }
                        }
                        if inventory_release
                            && !handled
                            && self.screen_state.screen_instance == initial_instance
                            && !self.key_focus_driving()
                            && self.observed_screen_view().is_some()
                        {
                            handled =
                                self.mounted_widget_mouse_released(self.cursor(), code, modifiers);
                        }
                    }
                    if had_screen {
                        self.settle_title_mode();
                        title_settled = true;
                    }
                    if let Some(receiver) = receiver {
                        handled = self.screen_button_post(
                            &receiver,
                            code,
                            pressed,
                            modifiers,
                            double_click,
                            handled,
                        );
                    }
                    if handled
                        && self.screen_state.screen_instance == initial_instance
                        && let Some(millis) = press_millis
                    {
                        self.screen_state
                            .record_click(millis, initial_instance, title_mode, code);
                    }
                    // Only the unhandled edge, with no screen now open, reaches mappings.
                    if handled
                        || self.mod_mappings_screen_open()
                        || self.remote_loading()
                        || self.reload_overlay.is_some()
                        || self.pending_entry.is_some()
                        || self.integrated_starting.is_some()
                        || self.pending_save.is_some()
                    {
                        return Ok(());
                    }
                    if pressed && !synthetic {
                        self.mod_mappings_button(button, true);
                    }
                    if pressed && !self.captured && !had_screen {
                        self.capture(true);
                        return Ok(());
                    }
                    let input = sunrise_client::keybinds::mouse_name(button);
                    if self.key_mapping_canceled(Some(input.as_str()), pressed) {
                        self.physical_buttons.remove(&button);
                        return Ok(());
                    }
                    if pressed && self.settings.keys.get("key.smoothCamera") == input {
                        self.smooth_camera_action();
                    }
                    self.mapped_mouse_event(button, &input, pressed)?;
                }
                WindowEvent::RedrawRequested => {
                    let flight_spans = self
                        .flight
                        .as_ref()
                        .filter(|flight| flight.raw)
                        .map(|_| sunrise_client::spans::record_frame());
                    sunrise_client::spans::time("recover_device", || self.recover_device())?;
                    self.observe_fullscreen();
                    self.sync_ime();
                    let now = Instant::now();
                    sunrise_threads::work::tick_start();
                    let elapsed = now.duration_since(self.previous);
                    self.frame_ticks = elapsed.as_secs_f32() * 20.0;
                    self.previous = now;
                    // `elapsed` is the interval that just ended: the wall time of the iteration
                    // before this one, booked now with the work it left, its own.
                    let booked = self.profile.unbooked.take();
                    if let Some(frame) = &booked {
                        self.record_perf_frame(
                            elapsed.as_secs_f32() * 1000.0,
                            frame.cpu_ms,
                            frame.gpu_ms,
                            &frame.spans,
                        );
                    }
                    self.upload_pace.frame(elapsed);
                    self.studio_fly(elapsed);
                    // Arrow-key camera controls belong to gameplay; chat and
                    // other screens use the same keys for their own navigation.
                    if self.screen == Screen::Playing
                        && self.title.is_none()
                        && self.sign_edit.is_none()
                        && self.book.is_none()
                        && !self.dialog_open()
                    {
                        let turn = |k| f32::from(u8::from(self.keys.raw(&k)));
                        let secs = elapsed.as_secs_f32().min(0.1);
                        let yaw = (turn(KeyCode::ArrowRight) - turn(KeyCode::ArrowLeft)) * 90.0;
                        let pitch = (turn(KeyCode::ArrowDown) - turn(KeyCode::ArrowUp)) * 90.0;
                        if yaw != 0.0 || pitch != 0.0 {
                            // 90 degrees a second, as `Entity.turn` degrees.
                            self.turn_player(
                                f64::from(yaw * secs / 0.15),
                                f64::from(pitch * secs / 0.15),
                            );
                        }
                    }
                    if self.title.is_none() {
                        sunrise_client::spans::time("update_world", || self.update_world());
                        sunrise_client::spans::time("spawn_player", || self.spawn_player())?;
                    } else if let Err(e) =
                        sunrise_client::spans::time("poll_remote", || self.poll_remote())
                    {
                        log::warn!("Sunrise: {e}");
                    }
                    let updated = Instant::now();
                    self.track_toggle_screens();
                    self.heal_hold_latches();
                    sunrise_hooks::engine::published(self.is_published());
                    self.update_window_title();
                    self.report_activity();
                    let target = self.tick_target_millis();
                    let steps = self.clock.advance(elapsed, target);
                    if steps > 0
                        && self.title.is_none()
                        && let Err(e) = self.poll_remote_before_tick()
                    {
                        log::warn!("Sunrise: {e}");
                    }
                    self.client_tick_steps(steps, Some(event_loop))?;
                    self.extras_frame(elapsed);
                    self.follow_retry()?;
                    self.apply_mouse_look();
                    self.input_look(elapsed.as_secs_f32());
                    self.advance_flight();
                    self.tick_world_icon();
                    let ticked = Instant::now();
                    let rendered = self.render_and_flip(elapsed)?;
                    self.native_frame_end()?;
                    // Drain once for both the raw flight and the slow-frame/capture route.
                    let frame_spans = if sunrise_client::spans::recording_here() {
                        sunrise_client::spans::take_raw()
                    } else {
                        Vec::new()
                    };
                    let block_entity_work = flight_spans.as_ref().map_or(
                        [0; 2],
                        sunrise_client::spans::FrameRecording::block_entity_work,
                    );
                    drop(flight_spans);
                    let mut own_work = None;
                    if let Some(r) = self.renderer.as_mut() {
                        let cpu = now.elapsed().saturating_sub(r.last_wait);
                        sunrise_threads::work::tick_end(cpu.as_nanos() as u64);
                        let ms = |d: Duration| d.as_secs_f32() * 1000.0;
                        let gpu = if rendered {
                            r.gpu_times.last().copied()
                        } else {
                            None
                        };
                        own_work = Some((ms(cpu), gpu));
                        if rendered {
                            let g = &self.settings.sunrise;
                            // The scale while Dynamic Resolution moves it on its own.
                            let dynamic_scale = g
                                .dynamic_resolution
                                .then(|| self.dynamic_resolution.scale(g.resolution_scale));
                            let times = sunrise_hooks::perf::FrameTimes::new(
                                ms(elapsed),
                                ms(cpu),
                                r.gpu_times.last().copied(),
                                dynamic_scale,
                            );
                            sunrise_hooks::engine::frame_timed(&times);
                            if let Some(&gpu) = r.gpu_times.last() {
                                self.gpu_utilization =
                                    f64::from(gpu) * 100.0 / f64::from(ms(cpu).max(0.001));
                            }
                            let settled =
                                self.screen == Screen::Playing && !self.streaming.terrain_loading();
                            frame_autotune::observe(
                                &self.settings.sunrise,
                                settled,
                                (&mut self.gpu_culling_auto, &mut self.dynamic_resolution),
                                r,
                                (ms(cpu), ms(elapsed)),
                            );
                            self.frame_stats.record_cpu(ms(cpu));
                            let render = r.last_encode;
                            self.frame_stats.record_phases([
                                ms(updated - now),
                                ms(ticked - updated),
                                ms(ticked.elapsed().saturating_sub(r.last_wait + render)),
                                ms(render),
                            ]);
                            self.frame_stats
                                .record_block_entities(r.static_block_entity_frame_stats());
                        }
                        self.frame_stats.record_gpu(&r.gpu_times);
                        r.gpu_times.clear();
                    }
                    self.limit_frame();
                    let paused = self.pauses_game();
                    if paused != self.pause {
                        let mut cancel = false;
                        self.native_client_pause_change_pre(paused, &mut cancel);
                        if !cancel {
                            self.pause = paused;
                            self.native_client_pause_change_post(paused);
                        }
                    }
                    if let Some(server) = &self.integrated {
                        server.set_paused(self.pause);
                        server.set_distances(
                            self.settings.render_distance,
                            self.settings.simulation_distance,
                            self.settings.entity_distance,
                        );
                    }
                    self.clock.end_frame(self.pause);
                    self.poll_screenshot_writes();
                    let mut consumed = !self.ui.screenshot_messages.is_empty();
                    let finished = self
                        .renderer
                        .as_mut()
                        .and_then(|r| r.screenshot_result.take())
                        .and_then(|result| match result {
                            Ok(pending) => self.save_screenshot(pending),
                            Err(error) => {
                                let result = Err(error);
                                self.finish_screenshot(&result, None);
                                Some(result)
                            }
                        });
                    if let Some(result) = &finished
                        && let Some(reply) = self.pending_screenshot.take()
                    {
                        let _ = reply.send(
                            match result {
                                Ok([w, h]) => {
                                    serde_json::json!({ "ok": true, "width": w, "height": h })
                                }
                                Err(e) => serde_json::json!({ "error": e }),
                            }
                            .to_string(),
                        );
                        consumed = true;
                    }
                    if let Some(result) =
                        self.renderer.as_mut().and_then(|r| r.capture_result.take())
                        && let Some(reply) = self.pending_render_capture.take()
                    {
                        let value = result.unwrap_or_else(|e| serde_json::json!({ "error": e }));
                        let _ = reply.send(value.to_string());
                    }
                    // A result nobody asked for was the world icon's
                    // (`tick_world_icon`): a failure tries again a second
                    // later, as `tryTakeScreenshotIfNeeded` does.
                    if !consumed && let Some(Err(error)) = finished {
                        log::warn!("Sunrise: couldn't save the world icon: {error}");
                    }
                    self.record_streaming_work(rendered, block_entity_work, &frame_spans);
                    if let Some((cpu_ms, gpu_ms)) = own_work {
                        // The next iteration books this one with the interval it ends in.
                        self.profile.unbooked = Some(profile_capture::UnbookedFrame {
                            cpu_ms,
                            gpu_ms,
                            spans: frame_spans,
                        });
                    }
                    if rendered {
                        self.frame_stats.record(elapsed.as_secs_f32() * 1000.0);
                        if sunrise_threads::alloc_count::ON {
                            self.record_frame_allocs();
                        }
                        self.debug_charts.frame_times.log(elapsed.as_nanos() as i64);
                        if self.frames == 0 {
                            // The launcher times Play to this line (https://github.com/sunrise-org/sunrise-launcher/blob/main/src/perf.rs).
                            log::info!(
                                "Sunrise: first frame after {} ms",
                                self.started.elapsed().as_millis()
                            );
                        }
                        if self.title.is_some() {
                            self.perf.title_shown();
                        }
                        if self.title.is_none()
                            && self.screen == Screen::Playing
                            && self.player.is_some()
                        {
                            // The interval is the last iteration's, and so are its spans.
                            self.perf.frame(
                                elapsed.as_secs_f32() * 1000.0,
                                booked.as_ref().map_or(&[], |frame| frame.spans.as_slice()),
                            );
                            if startup::filling() {
                                let mut fill: Vec<(&'static str, usize)> =
                                    self.streaming.stages().to_vec();
                                fill.push(("resident", self.world.chunks.len()));
                                fill.push(("uploads", self.pending_uploads.len()));
                                fill.push(("backlog", self.remote_backlog()));
                                startup::counters("world.fill", &fill);
                            }
                            if startup::on() {
                                startup::view_filled(
                                    self.world.chunks.len(),
                                    self.streaming.pending()
                                        + self.pending_uploads.len()
                                        + self.remote_backlog(),
                                    self.streaming.radius.max(0) as u32,
                                );
                            }
                            self.world_frames += 1;
                        }
                        self.frames += 1;
                        self.fps_frames += 1;
                        sunrise_client::crash::debug_crash_tick();
                        // `BlockableEventLoop.relayDelayCrash`: a worker's panic is reported on this thread.
                        sunrise_client::crash::relay_worker_panic();
                        if self.fps_at.elapsed() >= Duration::from_secs(1) {
                            self.fps = self.fps_frames as f32 / self.fps_at.elapsed().as_secs_f32();
                            self.fps_frames = 0;
                            self.fps_at = Instant::now();
                        }
                        if self.options.frames.is_some_and(|f| self.frames >= f)
                            || self
                                .options
                                .world_frames
                                .is_some_and(|f| self.world_frames >= f)
                        {
                            event_loop.exit();
                        }
                    }
                }
                _ => {}
            }

            Ok(())
        })();
        if let Some(post) = mouse_post
            && result.is_ok()
        {
            self.screen_mouse_post(post);
        }
        if let Err(e) = result {
            self.error = Some(e);
            event_loop.exit();
        }
        title_settled
    }

    /// Dispatch to the topmost screen only, taking the receiver's own result.
    fn screen_drag_response(
        &mut self,
        overlay: Option<&str>,
        button: MouseButton,
        framebuffer: [u32; 2],
        gui: (f32, f32),
    ) -> Result<bool> {
        let left = button == MouseButton::Left;
        if let Some(overlay) = overlay {
            return Ok(left && overlay == "dialog" && self.dialog_drag());
        }
        if !self.pad_synthetic_input() && self.container_drag() {
            return Ok(true);
        }
        if self.screen == Screen::Playing
            && let Some(title) = self.title.as_mut()
        {
            return Ok(left
                && (title.drag(self.cursor, gui)
                    || title.drag_field(&self.gui.atlas, self.cursor.0)));
        }
        Ok(match self.screen {
            Screen::BlockEditor if left => self.block_editor_drag(),
            Screen::Advancements if left => self.advancements_drag(),
            Screen::Stats if left => self.stats_drag(),
            Screen::DebugOptions if left => self.debug_options_drag(),
            Screen::HudEditor if left => self.hud_editor_drag(),
            Screen::WorldOptions if left => self.world_options_drag(),
            Screen::Options if left => self.options_drag()?,
            Screen::Creative => {
                let batch = HudBatch::new(framebuffer, self.settings.gui_scale_inputs());
                let (_, oy) = Self::creative_origin(&batch);
                (left && self.creative_screen.drag_scrollbar(self.cursor.1 - oy))
                    || self.inventory_drag_move()
            }
            Screen::Inventory => {
                (left && (self.station_drag() || self.recipe_book_drag()))
                    || self.inventory_drag_move()
            }
            _ => false,
        })
    }

    /// Route a winit key event and its text independently: Windows Alt-code
    /// characters can arrive without a physical key (`KeyboardHandler.charTyped`).
    fn dispatch_window_keyboard(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        key: Option<KeyCode>,
        pressed: bool,
        repeat: bool,
        shortcut: Option<sunrise_client::clipboard::EditShortcut>,
        text: Option<&str>,
    ) -> Result<()> {
        let has_screen = self.observed_screen_view().is_some();
        if let Some(key) = key
            && !self.dispatch_screen_key(
                event_loop,
                key,
                pressed,
                repeat,
                shortcut,
                Source::Keyboard,
            )?
            && !self.key_press_canceled(key, pressed, repeat)
        {
            self.native_panel_key_event(key, pressed, repeat);
            self.mapped_key_event(key, pressed, repeat)?;
        }
        // GLFW's character event follows the key, even if KeyPressed.Pre canceled.
        if pressed && has_screen {
            self.settle_title_mode();
            self.screen_chars_typed(text);
        }
        Ok(())
    }

    /// Shared window/socket ScreenEvent Pre → the captured screen → cancellable Post.
    /// A declined event is never offered to a replacement screen or its bindings.
    fn dispatch_screen_key(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        key: KeyCode,
        pressed: bool,
        repeat: bool,
        shortcut: Option<sunrise_client::clipboard::EditShortcut>,
        source: Source,
    ) -> Result<bool> {
        let instance = self.screen_state.screen_instance;
        let receiver = self.observed_input_screen();
        let canceled = if pressed {
            self.screen_key_pressed_on(receiver.as_ref(), key, repeat)
        } else {
            self.screen_key_released_on(receiver.as_ref(), key)
        };
        if canceled {
            if !pressed && source == Source::Keyboard {
                // Cleanup a physical release without dispatching it to another screen.
                self.remove_device_key(&key);
                self.mapped_key_event(key, false, false)?;
            }
            return Ok(true);
        }
        if self.screen_state.screen_instance == instance
            && self.screen_key_response(event_loop, key, pressed, repeat, shortcut)?
        {
            return Ok(true);
        }
        let post_canceled = self.screen_key_post_on(receiver.as_ref(), key, pressed, repeat);
        Ok(post_canceled
            || self.screen_state.screen_instance != instance
            || (pressed
                && (self.mounted_widget_captures_input()
                    || self.observed_overlay_now().is_some()
                    || self.title_input_active()
                    || matches!(
                        self.screen,
                        Screen::Chat
                            | Screen::Options
                            | Screen::Social
                            | Screen::WorldOptions
                            | Screen::CommandBlock
                            | Screen::BlockEditor
                            | Screen::DebugOptions
                            | Screen::DemoIntro
                            | Screen::Win
                    ))))
    }

    fn screen_key_response(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        key: KeyCode,
        pressed: bool,
        repeat: bool,
        shortcut: Option<sunrise_client::clipboard::EditShortcut>,
    ) -> Result<bool> {
        let instance = self.screen_state.screen_instance;
        let modifiers = glfw_modifiers(self.modifiers);
        if !pressed {
            if self.mounted_widget_key_released(key, modifiers) {
                return Ok(true);
            }
            if self.screen_state.screen_instance != instance
                || self.observed_overlay_now().is_some()
            {
                return Ok(false);
            }
            return match self.screen {
                Screen::Options => self.options_key_released(key),
                Screen::Win => self.win_key(key, false),
                _ => Ok(false),
            };
        }
        if !repeat && self.container_key(key) {
            return Ok(true);
        }
        if self.screen_state.screen_instance != instance {
            return Ok(false);
        }
        // AdvancementsScreen.keyPressed consumes its own binding before its children.
        if self.screen == Screen::Advancements
            && self.observed_overlay_now().is_none()
            && sunrise_client::keybinds::key_name(key)
                == Some(self.settings.keys.get("key.advancements"))
        {
            self.close_advancements();
            return Ok(true);
        }
        if key == KeyCode::Escape
            && self.screen == Screen::Mod
            && self.observed_overlay_now().is_none()
        {
            self.close_mod_screen();
            return Ok(true);
        }
        if self.mounted_widget_key_pressed(key, repeat, modifiers) {
            return Ok(true);
        }
        if self.screen_state.screen_instance != instance {
            return Ok(false);
        }
        let overlay = self.observed_overlay_now();
        let container =
            matches!(self.screen, Screen::Inventory | Screen::Creative) && overlay.is_none();
        if overlay == Some("dialog")
            && (!self.screen_state.widgets.has_focus() || key == KeyCode::Escape)
            && self.dialog_key(key, shortcut)?
        {
            return Ok(true);
        }
        if self.preset_editor_key(key) {
            return Ok(true);
        }
        let focus_handled = self.key_focus_key(event_loop, key);
        if let Some(handled) = focus_handled
            && (handled || !container)
        {
            return Ok(handled);
        }
        if self.screen_state.screen_instance != instance {
            return Ok(false);
        }
        // The real focused child has already had the key. A custom child's declined key
        // is not translated into a mouse click or passed to a vanilla field behind it.
        if !container && self.screen_state.widgets.has_focus() && key != KeyCode::Escape {
            return Ok(false);
        }
        match self.observed_overlay_now() {
            Some("command_prompt") => return Ok(self.command_prompt_key(key)),
            Some("link_prompt") => {
                if key == KeyCode::Escape {
                    self.answer_link_prompt(false);
                    return Ok(true);
                }
                return Ok(false);
            }
            Some("server_pack_prompt") => {
                if key == KeyCode::Escape {
                    self.answer_pack_prompt(false)?;
                    return Ok(true);
                }
                return Ok(false);
            }
            Some("code_of_conduct" | "dialog") => return Ok(false),
            _ => {}
        }
        if self.sign_edit.is_some() {
            return Ok(self.sign_edit_key(key, shortcut));
        }
        if self.book.is_some() {
            return Ok(self.book_key(key, shortcut));
        }
        if self.screen == Screen::Playing && self.title.is_some() {
            return self.title_screen_key(key, shortcut);
        }
        match self.screen {
            Screen::Chat => return Ok(self.chat_key(key, shortcut)),
            Screen::Social => return Ok(self.social_key(key)),
            Screen::CommandBlock => return Ok(self.command_block_key_code(key, shortcut, None)),
            Screen::BlockEditor => return Ok(self.block_editor_key_code(key, shortcut, None)),
            Screen::DebugOptions => return Ok(self.debug_options_key(key, shortcut)),
            Screen::Options => {
                if shortcut.is_some_and(|shortcut| self.options_shortcut(shortcut)) {
                    return Ok(true);
                }
                return self.options_key(key, None);
            }
            Screen::WorldOptions if key != KeyCode::Escape => {
                return Ok(self.world_options_key(key, shortcut));
            }
            Screen::Win => return self.win_key(key, true),
            Screen::Paused if self.pause_menu && key != KeyCode::Escape => {
                return self.pause_key(key);
            }
            Screen::Dead => return self.death_key(key),
            _ => {}
        }
        if container {
            return self.container_screen_key(key, shortcut);
        }
        if key == KeyCode::Escape && self.screen != Screen::Playing {
            self.key_pressed(key, key)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn title_screen_key(
        &mut self,
        key: KeyCode,
        shortcut: Option<sunrise_client::clipboard::EditShortcut>,
    ) -> Result<bool> {
        let field_focused = self.vanilla_text_field_focused();
        if let Some(shortcut) = shortcut
            && field_focused
        {
            self.title_shortcut(shortcut);
            return Ok(true);
        }
        let ctrl = self.has_control_down_with_quirk();
        let shift = self.has_shift_down();
        if let Some(sound) = self
            .title
            .as_mut()
            .and_then(|t| t.tab_bar_key(key, ctrl, shift))
        {
            if sound {
                self.play_audio_button_click();
            }
            return Ok(true);
        }
        let size = self.gui_size();
        let Some(title) = self.title.as_mut() else {
            return Ok(false);
        };
        match key {
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space
                if key != KeyCode::Space
                    || (!title.has_caret_field()
                        && (title.list_focused() || title.focused_button.is_some())) =>
            {
                let (action, sound) = title.confirm_key_size(size);
                let handled = sound || !matches!(action, Action::None);
                self.save_title_options();
                if sound {
                    self.play_audio_button_click();
                }
                self.title_action(action)?;
                Ok(handled)
            }
            KeyCode::Backspace
            | KeyCode::Delete
            | KeyCode::ArrowLeft
            | KeyCode::ArrowRight
            | KeyCode::Home
            | KeyCode::End
                if title.has_caret_field() =>
            {
                Ok(title.field_key(key, ctrl, shift))
            }
            KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::ArrowUp | KeyCode::ArrowDown => {
                title.arrow_key(key, size, shift);
                Ok(false)
            }
            key if title.page_missing_mods(key, size) => Ok(true),
            KeyCode::Backspace if field_focused => {
                self.title_edit(None, true);
                Ok(true)
            }
            KeyCode::Tab => {
                title.tab_dir(shift, size);
                Ok(false)
            }
            KeyCode::Escape => {
                title.escape();
                self.reopen_data_packs()?;
                self.save_title_options();
                Ok(true)
            }
            KeyCode::F5 if matches!(title.mode, sunrise_client::title::Mode::Multiplayer(_)) => {
                title.refresh_servers();
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}
/// misc/forcefield.png for the world border wall (square, power of two).
fn load_forcefield(root: &sunrise_assets::resource_source::ResourceSource) -> (u32, u32, Vec<u8>) {
    root.read("assets/minecraft/textures/misc/forcefield.png")
        .map_err(Into::into)
        .and_then(|bytes| sunrise_client::hud::decode_bytes(&bytes))
        .ok()
        .filter(|(w, h, _)| w == h && w.is_power_of_two())
        .unwrap_or((1, 1, vec![0; 4]))
}
impl<'a> Game<'a> {
    /// A game with no world open, over the loaded resources: the window's,
    /// or an integrated server's on its own thread.
    fn new(
        res: &'a Arc<Resources>,
        pack: PackResources,
        gui: UiResources,
        options: Options,
        settings: Settings,
        audio: Audio,
        prepared: Option<GameCpuResources>,
    ) -> Result<Self> {
        Self::construct(res, pack, gui, options, settings, audio, prepared, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn construct(
        res: &'a Arc<Resources>,
        pack: PackResources,
        gui: UiResources,
        options: Options,
        settings: Settings,
        audio: Audio,
        prepared: Option<GameCpuResources>,
        initial: Option<&mut Option<initial_client_models::InitialModelGeneration>>,
    ) -> Result<Self> {
        #[cfg(test)]
        Self::initialize_mod_runtime_for_tests();
        // A scoped hook on this thread asks this game's parts.
        mod_api::__private::mark_game_thread();
        let role = options.execution_role;
        let loop_side = role.loop_side();
        let item_data = res.core.item_data.context();
        // The window and its integrated server share the instance's one settings authority.
        let native_settings = options.mods.settings.clone();
        if role.is_logical_server() {
            // Config loading and server startup hooks can read settings before the world
            // publishes its loop context. An integrated server takes its own capture.
            let settings = if item_data.origin() == role.data_load_origin() {
                item_data.captured_loop_context(loop_side)
            } else {
                let inventory =
                    sunrise_mod_package::enabled::EnabledMods::read(&options.resources)?;
                let ids: std::collections::BTreeSet<_> = inventory.server_ids().collect();
                let registered = native_hooks::mod_settings::registered_setting_keys()
                    .filter(|(owner, _)| ids.contains(owner));
                item_data.loop_context_with_registered(loop_side, [], registered)
            };
            let server_owners: std::collections::BTreeSet<_> =
                sunrise_mod_package::enabled::EnabledMods::read(&options.resources)?
                    .server_ids()
                    .map(str::to_owned)
                    .collect();
            let _ = native_settings
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .sync_from_loop_context(&settings, |owner| server_owners.contains(owner));
        }
        let root = options.resources.clone();
        let mod_sources = options.mods.clone();
        let registry = pack.assets.registry;
        let level = Level::default();
        let save = Arc::new(SaveDir::new(
            std::env::current_dir()?
                .join("saves")
                .join(".sunrise-title"),
            sunrise_client::save::LevelShape::OVERWORLD,
            None,
        ));
        let prepared = prepared.map(Ok).unwrap_or_else(|| {
            GameCpuResources::load(
                &sunrise_assets::resource_source::ResourceRoot::new(
                    root.clone(),
                    packs::cache_dir(),
                    pack.source.clone(),
                ),
                None,
                pack.render.model_library.as_deref().ok(),
                !options.execution_role.is_dedicated_server(),
            )
        })?;
        let world = if options.execution_role.is_logical_server() {
            Game::authoritative_world_for_mods(res, 0)
        } else {
            Game::world_for_mods(res.facts().air, 0)
        };
        let tutorial = tutorial::Tutorial::new(tutorial::Step::from_option(
            settings.file.int("tutorialStep"),
        ));
        let creative = options.creative;
        let hud_layer_bounds = if options.execution_role.is_client() {
            use sunrise_client::gui_layers::{VanillaGuiLayer, order};
            Vec::with_capacity(
                order().map_or(VanillaGuiLayer::ALL.len(), |order| order.slots().len()),
            )
        } else {
            Vec::new()
        };
        let inventory = vec![None; 36];
        let now = Instant::now();
        let look = options.look;
        let slow_frame_ms = options
            .spikes
            .unwrap_or(sunrise_client::client_perf::SLOW_FRAME_MS);
        let particles = if options.execution_role.is_client() {
            let mut engine = sunrise_client::particles::ParticleEngine::new(
                &pack.source,
                &gui.atlas.sprites,
                &gui.atlas.translucent,
            );
            engine.set_block_state_source(Game::particle_block_states);
            engine
        } else if options.execution_role.is_dedicated_server() {
            sunrise_client::particles::ParticleEngine::without_sprites()
        } else {
            sunrise_client::particles::ParticleEngine::new_server(
                &pack.source,
                &gui.atlas.sprites,
                &gui.atlas.translucent,
            )
        };
        let resource_source = pack.source.clone();
        let (pack, gui) = if let Some(initial) = initial {
            let mut unpublished = pack.clone();
            unpublished.render = Arc::new(RenderAssets::unpublished());
            let context = &pack.context;
            unpublished.context = Arc::new(WorkerContext {
                assets: context.assets.clone(),
                render: unpublished.render.clone(),
                tables: context.tables.clone(),
                biomes: context.biomes.clone(),
                light: context.light.clone(),
                hidden_states: context.hidden_states.clone(),
                part_tables: context.part_tables.clone(),
            });
            unpublished.layer_names = Default::default();
            let metrics = UiResources::server(&pack.source)?;
            *initial = Some(initial_client_models::InitialModelGeneration { pack, gui });
            (unpublished, metrics)
        } else {
            (pack, gui)
        };
        // Until a world is opened the streamer is idle.
        let mut streaming = Streaming::new(
            Arc::new(TestTerrain::new(registry, res.biomes())?),
            pack.context.clone(),
            1,
        );
        streaming.paused = true;
        let standalone_geometry = native_block_entity_draw::StandaloneGeometry::new(
            &pack,
            &gui,
            res.try_block_entity_render(),
        );
        let mut game = Game {
            structure_spawns: sunrise_client::mobs::spawn_selection::StructureSpawns::load(
                &options.resources,
            ),
            world_biomes: None,
            world_biomes_base: None,
            res: res.clone(),
            rest_loader: None,
            pending_entry: None,
            debug_density: RefCell::default(),
            debug_fps_texts: RefCell::default(),
            pending_pack_reload: false,
            frame_attempted: render_settings::FrameAttempt::default(),
            pack_folder: None,
            pack_reload: None,
            reload_overlay: None,
            server_packs: Default::default(),
            client_reports: Arc::default(),
            link_prompt: None,
            command_prompt: None,
            code_of_conduct: None,
            dialogs: Default::default(),
            integrated_starting: None,
            world_renderer_ready: false,
            collision: res.collision(),
            motion: res.motion(),
            health_resources: res.motion().health.clone(),
            vehicle_contact_hurts: Vec::new(),
            damage_tag_generation: 1,
            data_map_registries: Vec::new(),
            pending_initial_models: None,
            stitching_block_atlas: None,
            pack,
            gui,
            instance_groups: Default::default(),
            prepared_items: Default::default(),
            standalone_geometry,
            fog: Default::default(),
            screen_fx: Default::default(),
            item_activation: Default::default(),
            weather_parameters: Default::default(),
            panorama_rotation: Default::default(),
            world,
            streaming,
            lod: None,
            lod_source_attempted: false,
            lod_observe: Default::default(),
            lod_level: None,
            lod_blue_map: None,
            lod_covered_at: None,
            view_arrival: Default::default(),
            options,
            player: None,
            drop_spam_throttler: Default::default(),
            spawn: None,
            clock: FixedStep::default(),
            previous_position: Vec3::ZERO,
            rope_rot_old: (0.0, 10.0),
            eye: 1.62,
            previous_eye: 1.62,
            yaw: look.map_or(level.rotation[0], |l| l.0),
            pitch: look.map_or(level.rotation[1], |l| l.1),
            inventory,
            selected: 0,
            carried: None,
            craft_grid: vec![None; 9],
            craft_size: 2,
            craft_table: None,
            inv_drag: None,
            container_buttons: HashSet::new(),
            container_owned_up: false,
            physical_presses: 0,
            tweaks: Default::default(),
            bundle_mouse: Default::default(),
            inv_clicks: Default::default(),
            last_inv_click: None,
            last_quick_moved: None,
            container: None,
            open_container_slots: None,
            station: None,
            potions: Default::default(),
            scratch_slot: None,
            mining: None,
            destroying: HashMap::new(),
            destroy_delay: 0,
            use_delay: 0,
            creative,
            mode: if creative {
                sunrise_client::command::args::GameMode::Creative
            } else {
                sunrise_client::command::args::GameMode::Survival
            },
            keys: Default::default(),
            physical_keys: HashSet::new(),
            rejected_keys: HashSet::new(),
            physical_buttons: HashSet::new(),
            drag_button: None,
            right_click_emulation: Default::default(),
            cursor: (0.0, 0.0),
            window_cursor: None,
            window_pointer_space: std::cell::OnceCell::new(),
            cursor_icon: sunrise_client::widgets::Cursor::Arrow,
            captured: false,
            capture_mode: None,
            regrab_on_refocus: false,
            screen_grab: false,
            pointer_grab_request: std::cell::Cell::new(None),
            dropped_files: Vec::new(),
            controller_notice: Default::default(),
            focused: true,
            window_hovered: false,
            window_occluded: false,
            unclaimed_window: false,
            unclaimed_modifiers: None,
            focus_after_run: None,
            ime_allowed: false,
            held_key_text: Default::default(),
            ime_area: None,
            focus_lost: None,
            modifiers: winit::keyboard::ModifiersState::empty(),
            fullscreen: sunrise_client::window::FullscreenState::new(false),
            video_modes: Default::default(),
            mouse: MouseLook::default(),
            agent_pointer: false,
            smooth_camera: false,
            vanilla_mappings: Default::default(),
            was_playing: false,
            pause: false,
            debug: false,
            gpu_passes: false,
            gpu_pass_detail: false,
            screen: Screen::Playing,
            screen_state: screen_build::ScreenState::default(),
            window: None,
            resize_state: None,
            renderer: None,
            mod_gpu: None,
            last_device_recovery: None,
            entity_vertices: Vec::new(),
            dynamic_lights: Vec::new(),
            luminance: Default::default(),
            see_through_vertices: Vec::new(),
            shadow_only_vertices: Default::default(),
            frame_batch_reserved_quads: Default::default(),
            entity_spans: Default::default(),
            entity_runs: Vec::new(),
            entity_motion: Default::default(),
            entity_motion_posed: Default::default(),
            skin_models: Default::default(),
            skin_frame: Default::default(),
            glowing_vertices: Default::default(),
            mod_sounds: Default::default(),
            mod_clear_particles: Default::default(),
            lent_level_random: None,
            pack_ids: None,
            health_hud: hud_layout::HealthHud::default(),
            started: now,
            spectator_gui: Default::default(),
            previous: now,
            last_input: now,
            pad: Default::default(),
            frames: 0,
            glint_frame: Default::default(),
            kept_block_entities: Default::default(),
            live_scenes: Default::default(),
            dynamic_textures: Default::default(),
            text_patches: Default::default(),
            prepared_model_draws: Default::default(),
            model_copy_order: Default::default(),
            world_geometry: Some(Default::default()),
            render_snapshot: std::cell::Cell::new(None),
            pack_frame_state: std::cell::Cell::new(None),
            pack_frame_slots: Default::default(),
            debug_renderers: Vec::new(),
            debug_renderers_version: None,
            hud_layer_next: None,
            hud_layer_bounds,
            hud_status_heights: Default::default(),
            hud_status_heights_set: Default::default(),
            mod_view_distances: Default::default(),
            block_entity_reach: Default::default(),
            item_level_cache: Default::default(),
            world_frames: 0,
            fps: 0.0,
            fps_frames: 0,
            fps_at: now,
            ticks: 0,
            gui_ticks: 0,
            texture_ticks: 0,
            error: None,
            message: None,
            save,
            level,
            player_autosaves: Default::default(),
            session_lock: None,
            world_adopted: false,
            startup_ticket_restore_pending: false,
            audio,
            loop_context: Arc::new(item_data.loop_context(loop_side, [])),
            tick: sunrise_client::parallel::TickThread::mint(),
            native_settings,
            damage_hooks: native_hooks::DAMAGE_HOOKS,
            paused_mob_hurt: None,
            music: {
                // The manager starts with the saved option (MusicManager's constructor).
                let mut music = sunrise_client::music::MusicManager::default();
                music.frequency = sunrise_client::music::MusicFrequency::parse(
                    &settings.file.name("musicFrequency"),
                );
                music
            },
            music_layers: Default::default(),
            // ToastManager.initializeMusicToast: Pause and Pause and Toast make the toast.
            now_playing: (settings.file.int("musicToast").clamp(0, 2) != 0).then(Default::default),
            music_toast_state: settings.file.int("musicToast").clamp(0, 2) as u8,
            music_notes: Default::default(),
            ambience: Default::default(),
            sound_paused: (false, false),
            mod_sounds_paused: false,
            cart_sounds: HashSet::new(),
            riding_cart_sounds: None,
            riding_sound: None,
            was_fall_flying: false,
            bee_sounds: HashMap::new(),
            guardian_look_scratch: Vec::new(),
            dropped: Default::default(),
            item_destroyed_damage: None,
            next_entity: 0,
            ticked_entities_through: u32::MAX,
            contact_drops: 0..0,
            particles,
            explicit_particle_quads: Default::default(),
            pickups: Default::default(),
            sign_edit: None,
            book: None,
            book_last_click: None,
            lectern_pulses: Vec::new(),
            loading_progress: 0.0,
            creation_read_started: false,
            loading_progress_at: None,
            moving_pistons_stale: false,
            spawn_origin: None,
            spawn_hook_fired: false,
            spawn_hook_cancelled: false,
            spawn_spiral: Default::default(),
            player_spawn: None,
            spawn_wait: Vec::new(),
            mobs: Default::default(),
            mob_index: Default::default(),
            creatures: Default::default(),
            sculk: Default::default(),
            raiding: Default::default(),
            attack_ticks: 100,
            attack_item: None,
            last_hurt_by_player: HashMap::new(),
            lead_killers: HashMap::new(),
            love_cause: HashMap::new(),
            died: Default::default(),
            be_anims: {
                let mut anims = sunrise_client::block_entity_anim::BlockEntityAnims::default();
                anims.classify(registry);
                anims
            },
            using: None,
            use_duration: 0,
            use_start: Default::default(),
            crossbow_sounds: [false; 2],
            view: Default::default(),
            limiter: sunrise_client::window::FramerateLimiter::new(0),
            frame_clock: Default::default(),
            scope_scale: 0.5,
            frame_ticks: 0.0,
            title: None,
            armor: [None, None, None, None],
            offhand: None,
            equipment: Default::default(),
            hotbar_pop: [0; 10],
            hand: Hand::Main,
            shield_cooldown: 0,
            cooldowns: Default::default(),
            command_source: None,
            native_command_filter: false,
            chat_broadcasts: Default::default(),
            titles: Default::default(),
            tool_highlight: Default::default(),
            saving_indicator: Default::default(),
            autosaving: false,
            // `computeNextAutosaveInterval` at the clock's 20 ticks a second.
            ticks_until_autosave: 6000,
            applied_modifiers: Vec::new(),
            experience: Experience::default(),
            player_score: 0,
            orbs: Default::default(),
            xp_delay: 0,
            ordered: Default::default(),
            level_up_sound: Default::default(),
            swing: Default::default(),
            swings_started: 0,
            predicted_swings: 0,
            heights: Default::default(),
            sway: Default::default(),
            visible_hands: [(0, None), (0, None)],
            hands_replaced: [false; 2],
            clicks: Default::default(),
            hand_strength: Default::default(),
            settings,
            session_video: SessionVideo::default(),
            look: Default::default(),
            tutorial,
            toast_manager: Default::default(),
            narration: Default::default(),
            options_from_title: false,
            avatar: Default::default(),
            flash: 0,
            custom_sky: Default::default(),
            rain_sound_time: 0,
            lightning_seen: [0; 2],
            sleeping: None,
            sleep_timer: 0,
            player_beds: Vec::new(),
            sleep_seen: (0, 0),
            dim: Dim::Overworld,
            world_root: PathBuf::new(),
            carrier_retention_snapshot: Vec::new(),
            owner_migration: None,
            world_flat: false,
            world_debug: false,
            server_trips: Vec::new(),
            level_teleports: Vec::new(),
            teleport_notes: Vec::new(),
            levels: Default::default(),
            level_pass: Default::default(),
            portals: Default::default(),
            respawn_config: None,
            end_portal_travel: false,
            win: None,
            demo_intro: None,
            end: Default::default(),
            block_ticks: Ticks::new(0),
            tick_scratch: TickScratch::default(),
            tick_chunks: Default::default(),
            tickets: Default::default(),
            random_tick_sections: Default::default(),
            level_events: Vec::new(),
            falling_vertices: Default::default(),
            falling: Default::default(),
            transient_blocks: Default::default(),
            paintings: Default::default(),
            painting_variants: prepared.painting_variants,
            item_frames: Default::default(),
            frame_model: prepared.frame_model,
            glow_frame_model: prepared.glow_frame_model,
            map_frame_model: prepared.map_frame_model,
            glow_map_frame_model: prepared.glow_map_frame_model,
            armor_stands: Default::default(),
            location_armor: Default::default(),
            cushions: Default::default(),
            others: Default::default(),
            armor_stand_assets: prepared.armor_stand_assets,
            client_item_extension_owners: Default::default(),
            waypoint_styles: prepared.waypoint_styles,
            waypoints: Default::default(),
            locator_camera: None,
            native_camera_frame: None,
            native_camera_entity_eye: None,
            posed_rotation: None,
            post_effects: Vec::new(),
            requested_post_effects_buf: Vec::new(),
            polytone_post_ticks: 0.0,
            expression_frame: Default::default(),
            spectated_post_effect: None,
            spectated_post_effect_active: false,
            experience_display: Default::default(),
            boats: Default::default(),
            leash_boats: Vec::new(),
            boat_models: prepared.boat_models,
            crystal_model: prepared.crystal_model,
            cushion_model: prepared.cushion_model,
            entity_models: prepared.entity_models,
            carts: Default::default(),
            watched_vehicles: Vec::new(),
            jukebox_sounds: HashMap::new(),
            bolts: Default::default(),
            maps: maps::Maps::default(),
            fight: dragon::Fight::default(),
            dragon: Default::default(),
            crystals: Default::default(),
            breath_clouds: Default::default(),
            dragon_model: prepared.dragon_model,
            map_patches: Vec::new(),
            name_tag_layouts: Default::default(),
            text_displays: Default::default(),
            see_through_collector: Default::default(),
            translucent_display_collector: Default::default(),
            display_visible_scratch: Vec::new(),
            block_display_parts_scratch: Vec::new(),
            display_inline_profiles_scratch: Vec::new(),
            interactions: Default::default(),
            remote_knots: Default::default(),
            knot_ids: Default::default(),
            broken_knots: Vec::new(),
            text_display_vertices: 0,
            skin_downloads: RefCell::new({
                let mut downloads = sunrise_client::skins::Downloader::new(
                    std::env::current_dir()
                        .unwrap_or_default()
                        .join("assets/skins"),
                );
                downloads.set_resource_root(resource_source.clone());
                downloads
            }),
            skin_profiles: RefCell::new(sunrise_client::skins::Resolver::new()),
            skin_slots: sunrise_client::skins::SkinSlots::new(sunrise_client::hud::SKIN_SLOTS),
            player_skin_slot: None,
            trim_slots: Default::default(),
            cart_models: prepared.cart_models,
            riding: None,
            riding_order: 0,
            started_riding: Vec::new(),
            rotation_forced: false,
            posed_rider: None,
            pending_vehicle: None,
            ender_items: vec![None; 27],
            ender_open: None,
            pending: Default::default(),
            spill: Default::default(),
            recipe_ui: Default::default(),
            mount_menu: Default::default(),
            hook: Default::default(),
            remote_hooks: Vec::new(),
            tridents: Default::default(),
            rockets: Default::default(),
            ominous_spawners: Default::default(),
            creaking: Default::default(),
            spawners: Default::default(),
            chunk_spawn_enemies: true,
            spawn_scratch: Default::default(),
            trident_model: prepared.trident_model,
            arrow_model: prepared.arrow_model,
            bee_stinger_model: prepared.bee_stinger_model,
            tnts: Default::default(),
            hoppers: HashMap::new(),
            transactions: Default::default(),
            dispense_source: None,
            slot_changed_active: Vec::new(),
            contents_changing: Vec::new(),
            mod_screen: None,
            mod_container_viewers: HashMap::new(),
            item_columns: None,
            pickup_columns: None,
            pass_mobs: None,
            spawner_tick_positions: Vec::new(),
            trial_spawner_tick_positions: Vec::new(),
            loop_states: Default::default(),
            creative_screen: prepared.creative_screen,
            creative_search_named: None,
            creative_contents: Default::default(),
            creative_ignore_text: false,
            control: None,
            agent_control_on: false,
            agent_last: None,
            frame_stats: Default::default(),
            alloc_mark: None,
            perf: Default::default(),
            pending_screenshot: None,
            screenshot_sequence: None,
            pending_render_capture: None,
            pending_bench: None,
            flight: None,
            ui_wait: Default::default(),
            events: Default::default(),
            input_trace: Default::default(),
            repro: Default::default(),
            profile: profile_capture::Profile::new(slow_frame_ms),
            tick_script: tick_script::TickScript::from_env(),
            agent: agent::Agent::default(),
            view_frustum: None,
            occlusion: Default::default(),
            async_particle_culler: RefCell::new(Default::default()),
            pending_uploads: Vec::new(),
            pending_uploads_merge: Vec::new(),
            pending_uploads_order: None,
            upload_pace: Default::default(),
            extras: Default::default(),
            pvp: Default::default(),
            mods: mod_sources,
            recipe_tables: Default::default(),
            panel: Default::default(),
            native_shaders: Default::default(),
            client_item_use: None,
            mod_keys: Default::default(),
            native_lifecycle: Default::default(),
            resident_entities: Default::default(),
            held_out: Vec::with_capacity(4),
            config_root: mod_api::__private::RegistrationData::resolved_config_dir(),
            native_configs: Default::default(),
            native_net: Default::default(),
            native_mod_screen: None,
            native_mod_screen_parent: false,
            sky_darken_memo: Default::default(),
            water_plane_memo: Default::default(),
            water_plane_scan: Default::default(),
            vehicle_mobs: Default::default(),
            persist: Default::default(),
            missing_professions: Default::default(),
            entity_attachments: Default::default(),
            collider_changes: Vec::new(),
            collider_placements: Default::default(),
            collider_histories: Default::default(),
            entity_initialization: Default::default(),
            attachment_indexes: Default::default(),
            remote: None,
            client_tasks: Default::default(),
            pending_client_tasks: Default::default(),
            realm_session: false,
            last_connection: None,
            connection_open: false,
            pending_retry: None,
            retry_due: None,
            lan_session: false,
            window_title: None,
            replay: Default::default(),
            studio: Default::default(),
            lan: None,
            integrated: None,
            level_shapes: Default::default(),
            serving: None,
            singleplayer_profile: None,
            world_options: None,
            options_to_world_options: false,
            command_block_screen: None,
            block_editor: None,
            structure_templates: HashMap::new(),
            test_highlights: HashMap::new(),
            game_tests: game_tests::GameTests::default(),
            #[cfg(feature = "friends")]
            friends: None,
            social: Default::default(),
            debug_keys: Default::default(),
            debug_charts: Default::default(),
            registered_debug_entries: sunrise_client::debug_entries::registered(),
            debug_entries: sunrise_client::debug_entries::DebugEntries::load(
                &std::env::current_dir().unwrap_or_default(),
            ),
            debug_options: None,
            memory_meter: Default::default(),
            debug_paint: Default::default(),
            debug_refresh: Default::default(),
            gpu_utilization: 0.0,
            gpu_culling_auto: sunrise_client::autotune::GpuCullingAuto::new(),
            dynamic_resolution: Default::default(),
            rendered_mobs: Default::default(),
            screen_blur_requested: Default::default(),
            spawn_counts: HashMap::new(),
            spawn_policies: HashMap::new(),
            passages: Default::default(),
            async_chunk_requests: Default::default(),
            saved_chunk_readers: Default::default(),
            detached_generators: Default::default(),
            pause_menu: true,
            pause_focus: None,
            death_focus: None,
            focus_screen: 0,
            previous_mode: None,
            gamemode_switcher: None,
            clipboard: Default::default(),
            user_input: Default::default(),
            text_selection: Default::default(),
            client_quit: std::cell::Cell::new(saving::ClientQuit::Running),
            saved_for_exit: false,
            pending_save: None,
            server_tick_deadline: None,
            frame_drawn: true,
            world_icon: None,
            forced_hit: None,
            tick_hit: Default::default(),
            tick_mob: None,
            entity_reach: 3.0,
            block_reach: 4.5,
            client_pick: Default::default(),
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
            kinetic_feedback: HashMap::new(),
            lan_global_events: Vec::new(),
            lid_openers: HashMap::new(),
            lan_acting: None,
            acting_abilities: None,
            native_commands: Default::default(),
            native_client_commands: Default::default(),
            lan_building_boxes: Default::default(),
            save_after_guest_turn: false,
            lan_guest_swaps: Vec::new(),
            lan_displaced_players: Vec::new(),
            interrupted_turns: Vec::new(),
            crash_save: Default::default(),
            lan_ender_menus: Vec::new(),
            player_state: Default::default(),
            active_player_breaks: Vec::new(),
            guest_packets: Vec::new(),
            guest_cooldowns: Vec::new(),
            guest_entity_events: Vec::new(),
            guest_equipment_breaks: Vec::new(),
            guest_block_sounds: Vec::new(),
            guest_player_sounds: Vec::new(),
            bound_sounds: Vec::new(),
            lan_acting_name: None,
            lan_acting_uuid: None,
            tnt_refused: None,
            lan_acting_permission: None,
            lan_acting_spawn_protection: None,
            lan_acting_entity: None,
            lan_acting_fake: false,
            fake_action: Default::default(),
            chat_capture: None,
            admin_feedback: Vec::new(),
            console_broadcast: None,
            rcon_source: false,
            server_lists: None,
            user_cache: None,
            world_tables: None,
            server_loot: Default::default(),
            server_items: None,
            cmds: Default::default(),
            reload_resources: Default::default(),
            ui: chat::UiState::new(),
            combat: Default::default(),
            guest_combat: Default::default(),
            mount: Default::default(),
            stats: Default::default(),
            score: Default::default(),
            fns: Default::default(),
            bosses: Default::default(),
            boss_overlay: Default::default(),
            exec: Default::default(),
            rand_sequences: Default::default(),
            brushing: Default::default(),
            border: Default::default(),
            border_renderer: Default::default(),
            border_section: None,
            border_state: Default::default(),
            tab_list: Default::default(),
            subtitles: Default::default(),
            adv: Default::default(),
            restore_player_state: true,
            stats_ui: Default::default(),
            adv_ui: Default::default(),
            links_ui: Default::default(),
            camera_type: Default::default(),
            requested_camera_type: Default::default(),
            fangs: Default::default(),
            player_model: None,
        };
        game.reset_entity_residency();
        Ok(game)
    }
}

/// `key.hotbar.1` to `key.hotbar.9` in a menu.
const HOTBAR_DIGITS: [KeyCode; 9] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
];
/// The other keys `InputConstants.Key.getNumericKeyValue` gives a digit.
const NUMERIC_KEYS: [KeyCode; 11] = [
    KeyCode::Digit0,
    KeyCode::Numpad0,
    KeyCode::Numpad1,
    KeyCode::Numpad2,
    KeyCode::Numpad3,
    KeyCode::Numpad4,
    KeyCode::Numpad5,
    KeyCode::Numpad6,
    KeyCode::Numpad7,
    KeyCode::Numpad8,
    KeyCode::Numpad9,
];

/// A title-shaped frame: black clear and panorama, only the HUD is drawn.
/// The window's `HWND`, for the boot screen's GDI paint (`sunrise_threads`):
/// the raw handle read is safe, the paint is not.
#[cfg(target_os = "windows")]
fn hwnd(window: &Window) -> Option<isize> {
    use winit::raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    match window.window_handle() {
        Ok(raw) => match raw.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
            _ => None,
        },
        Err(_) => None,
    }
}

pub use launch_api::{UsageError, exit_code};

pub(crate) fn run_with(
    options: Options,
    native: sunrise_mod_package::runtime::Runtime,
) -> Result<()> {
    let mut options = Options::read(options)?;
    options.mods.native = Arc::new(native);
    if options.execution_role == execution_role::ExecutionRole::DedicatedServer
        && !dedicated::preflight(&mut options)?
    {
        return Ok(());
    }
    native_hooks::install_block_contact_hooks();
    // Before any worker starts: on Linux they go behind the server thread
    // and the render thread.
    sunrise_threads::put_helpers_behind();
    if options.execution_role.is_client() {
        // The render thread, like the server thread, ahead of the workers on
        // macOS and Windows. While terrain loads, the workers
        // (sunrise_threads::jobs) and other programs outnumber the cores, and a frame that gave its core up (in a GPU submit) waited
        // 10 to 200 ms to get one back.
        sunrise_threads::favour_current();
    }
    // A dedicated server too: vanilla's "Server Shutdown Thread" (`DedicatedServer.initServer`'s
    // shutdown hook) halts the server and saves its players and levels on SIGTERM, SIGINT or SIGHUP.
    if options.execution_role.is_client()
        || options.execution_role == execution_role::ExecutionRole::DedicatedServer
    {
        sunrise_threads::catch_stop_signals();
    }
    let started = Instant::now();
    // A dedicated server times its spans always, so its slow ring holds the steps of a lag spike (microseconds a tick).
    if options.spikes.is_none()
        && options.execution_role == execution_role::ExecutionRole::DedicatedServer
    {
        options.spikes = Some(dedicated::SLOW_TICK_MS);
    }
    tick_profile::want_spans(tick_profile::SPANS_SPIKES, options.spikes.is_some());
    // A player who allows performance statistics also says what their slow frames spent their
    // time in: this thread, which draws the frames, times its steps (wall time only).
    if options.execution_role.is_client()
        && sunrise_telemetry::Consent::load(&sunrise_client::logging::game_home())
            .allows(sunrise_telemetry::Kind::Perf)
    {
        sunrise_client::spans::record_here(true);
    }
    let vanilla_root = options.resources.clone();
    // Saves from before 1.13 upgrade with vanilla's tables, prepared from the player's jar.
    sunrise_world::datafix::set_resources(
        &std::path::absolute(&vanilla_root).unwrap_or_else(|_| vanilla_root.clone()),
    );
    let preparation = match preflight_resources(
        &vanilla_root,
        sunrise_hooks::engine::get_the_game(),
        options.execution_role.is_dedicated_server(),
    ) {
        Ok(profile) => profile,
        Err(error) => {
            // Keep the introductory GUI for an installation with no manifest.
            // A present but unusable preparation needs the exact re-prepare error.
            if !options.offscreen
                && !synchronous_startup(&options)
                && !vanilla_root
                    .join(sunrise_core::prepared_profile::MANIFEST)
                    .is_file()
            {
                log::error!("Sunrise: {error}");
                let settings = Settings::load(&Game::options_path());
                return no_resources::show(&vanilla_root, &settings);
            }
            return Err(error.into());
        }
    };
    let mut early_settings = Settings::load(&Game::options_path());
    options
        .mods
        .capture_code_switches(&early_settings.mods.disabled);
    options
        .mods
        .select(&std::env::current_dir()?, &early_settings.mods.enabled);
    mods::validate_startup_packages(&options.mods)?;
    let jar_store = sunrise_client::logging::game_home().join("jars");
    let admission = options.mods.admit_startup_packages(&jar_store)?;
    api_values::install_packages(admission.native_packages(), admission.content_packages())?;
    let instance_dir = Game::game_dir();
    if let Err(error) = native_hooks::config_dir(&instance_dir) {
        log::warn!("Sunrise: config folder: {error}");
    }
    mods::install_registration_data(
        &vanilla_root,
        instance_dir,
        mods::runtime_files::sources(&options.mods),
        mods::package_files::sources(&admission),
    )?;
    {
        crate::game::part_admission::admit_parts(mod_api::__private::mods())?;
        // The structure types those mods registered, handed to the engine before
        // any world opens, as its StructureRegistry reads them.
        if sunrise_world::worldgen::structure::mod_types::install_mod_structure_types(
            native_structure_types::installed_structure_types(),
        )
        .is_err()
        {
            log::warn!("Mod structure types already installed; keeping the first table");
        }
        // Their pool element types, before any world decodes its template pools.
        if sunrise_world::worldgen::structure::mod_elements::install_mod_pool_element_types(
            native_pool_elements::installed_pool_element_types(),
        )
        .is_err()
        {
            log::warn!("Mod pool element types already installed; keeping the first table");
        }
        // Their structure processor types, before any world decodes its processor lists.
        if sunrise_world::worldgen::structure::mod_processors::install_mod_structure_processor_types(
            native_structure_processors::installed_structure_processor_types(),
        )
        .is_err()
        {
            log::warn!("Mod structure processor types already installed; keeping the first table");
        }
        // Their rule test and position rule test types, before any processor list or feature decodes.
        if sunrise_world::worldgen::mod_rule_tests::install_mod_rule_test_types(
            native_rule_tests::installed_rule_test_types(),
        )
        .is_err()
        {
            log::warn!("Mod rule test types already installed; keeping the first table");
        }
        if sunrise_world::worldgen::mod_rule_tests::install_mod_pos_rule_test_types(
            native_rule_tests::installed_pos_rule_test_types(),
        )
        .is_err()
        {
            log::warn!("Mod position rule test types already installed; keeping the first table");
        }
        // Their feature types, before any world decodes its configured features.
        if sunrise_world::worldgen::mod_features::install_mod_feature_types(
            native_feature_types::installed_feature_types(),
        )
        .is_err()
        {
            log::warn!("Mod feature types already installed; keeping the first table");
        }
        // Their placement modifier types, before any world decodes its placed features.
        if sunrise_world::worldgen::placement::mod_placements::install_mod_placement_modifier_types(
            native_placement_modifiers::installed_placement_modifier_types(),
        )
        .is_err()
        {
            log::warn!("Mod placement modifier types already installed; keeping the first table");
        }
        // The levels they registered, below the data packs of every world opened.
        if sunrise_world::worldgen::overrides::install_mod_dimensions(
            native_dimensions::installed_dimensions(),
        )
        .is_err()
        {
            log::warn!("Mod dimensions already installed; keeping the first table");
        }
        // Their chunk generators and biome sources, which those levels' and the packs' stems name.
        native_chunk_generator::install();
        native_biome_source::install();
        mods::register_components(&vanilla_root)?;
        sunrise_client::item_sync::install_consume_effect_codecs()?;
        mods::register_standalone_models(options.execution_role);
        mods::register_layer_definitions(options.execution_role, &vanilla_root)?;
        mods::register_debug_entries(options.execution_role);
        mods::register_gui_layers(options.execution_role);
        mods::register_client_tooltip_component_factories(options.execution_role);
        mods::register_map_decoration_types();
        mods::register_range_select_item_model_properties(options.execution_role);
        mods::register_select_item_model_properties(options.execution_role);
        mods::register_resource_sources(options.execution_role);
        mods::register_enchantment_effects();
        mods::register_client_presentations(options.execution_role, &vanilla_root)?;
        mods::register_meshing(options.execution_role);
        mods::register_particles(options.execution_role)?;
        mods::freeze_declarations();
    }
    // Minecraft.<init>: read the last start's flag, then mark this one as not
    // finished until onGameLoadFinished. A dedicated server has no options.txt.
    if options.execution_role.is_client() {
        if unclean_start_recovery(&mut early_settings) {
            log::warn!(
                "Detected unexpected shutdown during last game startup: resetting fullscreen mode"
            );
        }
        early_settings.file.set(
            "startedCleanly",
            sunrise_client::options_file::V::Bool(false),
        );
        early_settings.save(&Game::options_path());
    }
    // Minecraft's constructor opens the narrator; a server has none.
    if options.execution_role.is_client() {
        narrator::start(&early_settings);
    }
    // Someone is playing: the anonymous ping behind the launcher's "N playing now", and the
    // official server in their list (servers.dat in the game folder, the working directory). Not
    // for a server, a benchmark, a run that quits by itself, a harness's fixture or a run that
    // sunrise-ctl drives: those start in a source checkout and wrote the list there.
    if !options.offscreen && !synchronous_startup(&options) {
        let version = sunrise_client::telemetry::version();
        let game_dir = sunrise_client::logging::player_dir();
        sunrise_hooks::engine::started(&sunrise_hooks::Started::new(&version, &game_dir));
    }
    // A game window's hooks (Discord's presence): never for a server.
    activity::client_started(&options);
    // Snapshots of derived tables live in the game folder, never beside the
    // shared resources.
    Behaviours::set_cache_dir(std::env::current_dir()?.join(".sunrise-cache"));
    if !synchronous_startup(&options) {
        return run_loading(options, early_settings, vanilla_root, None, started);
    }
    let roots = if options.execution_role.is_dedicated_server() {
        // Dedicated startup sees enabled mod assets but no client's resource packs.
        let base = mods::content_root(&vanilla_root, &options.mods)?;
        match preparation {
            sunrise_core::prepared_profile::Profile::Full => base,
            sunrise_core::prepared_profile::Profile::Server => base.with_server_profile(),
        }
        .into()
    } else {
        packs::startup_root(
            &vanilla_root,
            &early_settings.resource_packs,
            &[],
            &options.mods,
        )?
    };
    let mut loaded = load_resources_sync(
        roots,
        pack_finders::data_packs(&options.mods),
        StartupLoadContext {
            vanilla_root: &vanilla_root,
            launch_world: launch_world(&options),
            started,
        },
        &mut early_settings,
        mods::LoadRole::startup(options.execution_role),
        &Progress::new(),
        &options.mods,
    )?;
    let gui = loaded.gui.ok_or("missing menu bundle")?;
    options.resources = loaded.root.clone();
    let audio = if options.execution_role.is_dedicated_server() {
        Audio::for_server()
    } else if !options.execution_role.is_client() || options.headless {
        Audio::silent(&loaded.pack.source)
    } else {
        Audio::with_device(&loaded.pack.source, &early_settings.sound_device)
    };
    let resources = loaded.resources;
    let game = start_game(
        &resources,
        loaded.pack,
        gui,
        options,
        loaded.pack_error,
        audio,
        vanilla_root,
        started,
        loaded.rest_loader,
        early_settings,
        None,
        std::mem::take(&mut loaded.client_applies),
        loaded.entity_models,
        None,
    )?;
    if let Some(mut game) = game {
        // Minecraft.onGameLoadFinished: a start that made it this far is clean.
        if game.options.is_client_start() && game.options.headless {
            game.settings.file.set(
                "startedCleanly",
                sunrise_client::options_file::V::Bool(true),
            );
            game.settings.save(&Game::options_path());
        }
        run_event_loop(game)?;
    }
    Ok(())
}

/// The right and left arms of a player model layer (`PlayerModel.rightArm`
/// and `leftArm`, with their sleeves), each keeping the layer.
fn player_arms(
    root: &std::path::Path,
    key: &str,
    document: Option<&sunrise_client::mobs::EntityModelDocument>,
) -> [Option<sunrise_client::mobs::Part>; 2] {
    let model = match document {
        Some(document) => document.model(key),
        None => sunrise_client::mobs::load_model(root, key),
    };
    ["right_arm", "left_arm"].map(|name| model.as_ref().and_then(|m| m.detached_child(name)))
}

// Player.tick: cap the sleeping fade at 100, then fade out over 10 waking ticks.
fn tick_sleep_timer(timer: i32, sleeping: bool) -> i32 {
    if sleeping {
        timer.wrapping_add(1).min(100)
    } else if timer > 0 {
        let next = timer.wrapping_add(1);
        if next >= 110 { 0 } else { next }
    } else {
        timer
    }
}

/// Where a ray entering `pos` first meets the block's outline boxes, if it
/// does: a hollow block (torch, fence, flower) lets the ray through.
fn outline_stop(
    origin: [f64; 3],
    dir: [f64; 3],
    reach: f64,
    pos: [i32; 3],
    face: [i32; 3],
    boxes: impl FnOnce() -> Vec<[f64; 6]>,
) -> Option<Hit> {
    let (t, face) = interaction_shapes::entry(origin, dir, &boxes(), reach, face)?;
    Some(Hit {
        pos,
        face,
        point: std::array::from_fn(|i| origin[i] + dir[i] * t),
        inside: false,
    })
}
/// `VoxelShape.clip` of a fluid source's shape for a ray that starts in the
/// source's own block `cell`: the face and point it hits, in `pick_fluid`'s
/// `Hit` terms. `FlowingFluid.getShape` is the whole block under the same
/// fluid and `getOwnHeight` (8/9) tall otherwise. A start inside the shape
/// (tested 1e-7 of the way along) hits at the start, on the face opposite
/// `Direction.getApproximateNearest(dir)`; from above it, a ray going down
/// enters through the top while still in this block.
fn clip_start_source(
    eye: [f64; 3],
    dir: [f64; 3],
    reach: f64,
    cell: [i32; 3],
    same_above: bool,
) -> Option<([i32; 3], [f64; 3])> {
    let height = if same_above {
        1.0
    } else {
        f64::from(8.0f32 / 9.0f32)
    };
    let probe_y = eye[1] + dir[1] * reach * 1.0e-7;
    if probe_y - f64::from(cell[1]) < height {
        // Direction.values() order; the first of equal dot products wins.
        let normals: [[i32; 3]; 6] = [
            [0, -1, 0],
            [0, 1, 0],
            [0, 0, -1],
            [0, 0, 1],
            [-1, 0, 0],
            [1, 0, 0],
        ];
        let mut nearest = normals[2];
        let mut best = f64::NEG_INFINITY;
        for n in normals {
            let dot =
                f64::from(n[0]) * dir[0] + f64::from(n[1]) * dir[1] + f64::from(n[2]) * dir[2];
            if dot > best {
                best = dot;
                nearest = n;
            }
        }
        return Some((nearest.map(|v| -v), eye));
    }
    if dir[1] >= 0.0 {
        return None;
    }
    let t = (f64::from(cell[1]) + height - eye[1]) / dir[1];
    if t > reach {
        return None;
    }
    let point = [
        eye[0] + dir[0] * t,
        eye[1] + dir[1] * t,
        eye[2] + dir[2] * t,
    ];
    (point[0].floor() as i32 == cell[0] && point[2].floor() as i32 == cell[2])
        .then_some(([0, 1, 0], point))
}

fn reset_world_progress(adv: &mut advancements::AdvState, stats: &mut stats::Stats) {
    *adv = Default::default();
    *stats = Default::default();
}

pub(crate) use closest_space::closest_space_nudge;
#[cfg(test)]
use inventory_input::drop_whole_stack;
use subtitles::{Subtitles, in_game_ui, subtitles};

#[cfg(test)]
use hud_render::block_outline_drawn;
use hud_render::{debug_crosshair_drawn, is_game_master_block};
#[cfg(test)]
mod test_neighbourhood {
    //! Shared fixture step for tests that tick entities on prepared data.
    use super::*;

    impl Game<'_> {
        /// Loads the chunks within 2 columns of `(cx, cz)` that are not loaded yet.
        /// Entity ticking waits for every chunk in that 5x5 neighbourhood
        /// (`ChunkMap.prepareEntityTickingChunk`), so a fixture that ticks mobs,
        /// items or TNT in a chunk calls this after setting up its own chunks.
        pub(crate) fn load_ticking_neighbourhood(&mut self, cx: i32, cz: i32) {
            for z in cz - 2..=cz + 2 {
                for x in cx - 2..=cx + 2 {
                    if self.world.chunks.contains_key(&(x, z)) {
                        continue;
                    }
                    self.world.chunks.insert(
                        (x, z),
                        std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(x, z)),
                    );
                    // A chunk that is already registered keeps its ticks.
                    let _ = self
                        .block_ticks
                        .add_empty_chunk(sunrise_core::position::ChunkPos::new(x, z));
                }
            }
        }
    }
}

/// A server's data load with no mods and no settings, for loaders under test.
#[cfg(test)]
fn test_server_load() -> sunrise_world::native_load_context::LoadContext {
    use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs};
    LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    )
}

#[cfg(test)]
mod nested_orb_spawn_tests {
    #[test]
    #[ignore = "daily"]
    fn experience_pickup_keeps_nested_orbs_until_the_next_ordered_pass() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            game.block_ticks
                .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
                .unwrap();
            for (x, value) in [(7.5, 1), (9.5, 3), (11.5, 7)] {
                game.add_orb([x, 80.0, 3.5], value);
            }
            let _ = game.orbs.remove(1);
            let mut expected: Vec<_> = game.orbs.iter().map(|o| (o.id, o.join)).collect();
            crate::game::entity::taken_pass_tests::with_spawn(
                |g| g.add_orb([3.5, 80.0, 3.5], 1),
                || game.tick_experience(),
            );
            assert_eq!(game.orbs.len(), 3);
            let join = game.orbs[2].join;
            expected.push((game.orbs[2].id, join));
            assert_eq!(
                game.orbs.iter().map(|o| (o.id, o.join)).collect::<Vec<_>>(),
                expected
            );
            assert_eq!(game.orbs[2].age, 0);
            game.ordered.announced = sunrise_client::join_seq::JoinSeq::MAX;
            game.tickets.update(None);
            game.begin_ordered_ticks();
            game.run_ordered_ticks(sunrise_client::join_seq::JoinSeq::MAX, false)
                .unwrap();
            let spawned = game.orbs.iter().find(|o| o.join == join).unwrap();
            assert_eq!(spawned.age, 1);
            assert_eq!(
                game.orbs.iter().map(|o| (o.id, o.join)).collect::<Vec<_>>(),
                expected
            );
        });
    }
}

#[cfg(test)]
pub(crate) fn in_own_process(name: &str, body: impl FnOnce()) {
    const OWN_PROCESS: &str = "SUNRISE_TEST_OWN_PROCESS";
    if std::env::var_os(OWN_PROCESS).is_some_and(|v| v == name) {
        return body();
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--include-ignored", "--test-threads=1"])
        .env(OWN_PROCESS, name)
        .output()
        .unwrap();
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success() && report.contains("test result: ok. 1 passed;"),
        "{name} failed in its own process:\n{report}"
    );
}

impl Game<'_> {
    fn presence_status(&self) -> sunrise_client::social::PresenceStatus {
        use sunrise_client::social::PresenceStatus;
        if self.integrated.is_some() {
            // `getSingleplayerServer().getMultiplayerScope()`: LAN while the world is
            // opened to LAN (a pause-menu Open to LAN or an invite code).
            if self.lan.is_some() {
                PresenceStatus::PlayingHostedServer
            } else {
                PresenceStatus::PlayingOffline
            }
        } else if self.remote.is_some() {
            // `getCurrentServer().isRealm()`: the connection came from Realms.
            if self.realm_session {
                PresenceStatus::PlayingRealms
            } else {
                PresenceStatus::PlayingServer
            }
        } else {
            PresenceStatus::Online
        }
    }
}

#[cfg(test)]
#[path = "game/interface/tests/sign_chain_tests.rs"]
mod sign_chain_tests;

#[cfg(test)]
#[path = "game/interface/tests/sign_outline_tests.rs"]
mod sign_outline_tests;

#[cfg(test)]
#[path = "game/interface/tests/sign_use_tests.rs"]
mod sign_use_tests;
