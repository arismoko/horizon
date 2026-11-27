//! The playable game loop: streaming world, player simulation at 20 ticks/s,
//! block interaction, inventory and HUD.

#[allow(unused_imports)]
pub use resource_load::{
    Progress, WorldRestLoader, launch_world, named_scope, synchronous_startup,
};
/// The content tables the world plays with, as field-level borrows: the
/// world's data packs' over the game's own. The `Game::items`/`recipes`/
/// `stations` methods read the same tables, but taking `&self` blocks a
/// `&mut self.…` in the same scope; these expand to a match on the fields,
/// which borrows stay disjoint from.
#[doc(hidden)]
#[macro_export]
macro_rules! world_items {
    ($s:ident) => {
        match (
            &$s.server_items,
            $crate::game::Side::world_tables({
                use $crate::game::SideGame as _;
                ($s).side_state()
            }),
        ) {
            (Some(items), _) => &**items,
            (None, Some(t)) => &t.items,
            (None, None) => $s.res.items(),
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! world_variants {
    ($s:expr) => {
        match $crate::game::Side::world_tables({
            use $crate::game::SideGame as _;
            ($s).side_state()
        }) {
            Some(t) => &*t.variants,
            None => ($s).res.variants(),
        }
    };
}
#[doc(hidden)]
#[macro_export]
macro_rules! world_recipes {
    ($s:ident) => {
        match $crate::game::Side::world_tables({
            use $crate::game::SideGame as _;
            ($s).side_state()
        }) {
            Some(t) => &t.recipes,
            None => $s.res.recipes(),
        }
    };
}

/// The painting variants the world places with (`Painting.create`): its own
/// registry and `#minecraft:placeable`, else the prepared root's.
#[doc(hidden)]
#[macro_export]
macro_rules! world_paintings {
    ($s:expr) => {
        match $crate::game::Side::world_tables({
            use $crate::game::SideGame as _;
            ($s).side_state()
        }) {
            Some(t) => &t.painting_variants,
            None => &($s).painting_variants,
        }
    };
}
#[doc(hidden)]
#[macro_export]
macro_rules! world_stations {
    ($s:ident) => {
        match $crate::game::Side::world_tables({
            use $crate::game::SideGame as _;
            ($s).side_state()
        }) {
            Some(t) => &t.stations,
            None => $s.res.stations(),
        }
    };
}
/// The biome table the world plays with (`Game::biomes`), as a field-level
/// borrow: the world's packs' over the game's own.
#[doc(hidden)]
#[macro_export]
macro_rules! world_biomes {
    ($s:expr) => {
        match &$s.world_biomes {
            Some(b) => b,
            None => $s.res.biomes(),
        }
    };
}
/// The acting player's stacks as field-level borrows, in vanilla `EquipmentSlot` order.
#[doc(hidden)]
#[macro_export]
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
#[doc(hidden)]
#[macro_export]
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
                    $crate::game::Side::levels_generation(
                        {
                            use $crate::game::SideGame as _;
                            ($s).side_state()
                        },
                        id,
                    ),
                ))
            }),
            facts: $s.res.facts(),
            tables: $crate::game::native_block_tables::BlockTables::new(&$s.res, &$s.pack),
            biomes: $crate::world_biomes!($s),
            behaviours: $s.res.behaviours(),
            features: $crate::game::side_ops::tables::SideOps::side_level_features(
                {
                    use $crate::game::SideGame as _;
                    ($s).side_state()
                },
                $s.level.enabled_features.as_arc(),
            ),
            items: $s.res.items(),
            damage_types: &$s.health_resources.damage,
            player_equipment: $crate::acting_equipment!($s),
            hit_borrows: $crate::game::native_surface::HitBorrows {
                living_types: $s.res.living_type_keys(),
                attributes: &$s.motion.attributes,
                player_effects: &$s.potions.effects,
                player_uuid: $crate::game::Side::lan_acting_uuid({
                    use $crate::game::SideGame as _;
                    ($s).side_state()
                }),
                player_attachments: &$s.player_state.attachments,
                entity_attachments: &*$s.entity_attachments,
            },
            tag_items: $crate::game::side_ops::tables::SideOps::side_game_tag_items(
                {
                    use $crate::game::SideGame as _;
                    ($s).side_state()
                },
                &$s.res,
            ),
        }
    };
    ($s:expr, $meta:expr) => {{
        let (player_uuid, level, session_lock): (Option<u128>, Option<mod_api::LevelRef>, bool) =
            $meta;
        $crate::game_side!(
            $s,
            (player_uuid, level, session_lock),
            $crate::game::side_ops::tables::SideOps::side_level_features(
                $crate::game::SideGame::side_state($s),
                $s.level.enabled_features.as_arc()
            ),
            if $s.options.execution_role.is_client() {
                $crate::game::side_ops::tables::SideOps::side_game_tag_items(
                    $crate::game::SideGame::side_state($s),
                    &$s.res,
                )
            } else {
                session_lock.then(|| $s.res.items())
            }
        )
    }};
    ($s:expr, $meta:expr, $features:expr, $tags:expr) => {{
        let (player_uuid, level, _session_lock): (Option<u128>, Option<mod_api::LevelRef>, bool) =
            $meta;
        $crate::game::native_surface::GameSide {
            dim: $s.dim,
            level,
            facts: $s.res.facts(),
            tables: $crate::game::native_block_tables::BlockTables::new(&$s.res, &$s.pack),
            biomes: $crate::world_biomes!($s),
            behaviours: $s.res.behaviours(),
            features: $features,
            items: $s.res.items(),
            damage_types: &$s.health_resources.damage,
            player_equipment: $crate::acting_equipment!($s),
            hit_borrows: $crate::game::native_surface::HitBorrows {
                living_types: $s.res.living_type_keys(),
                attributes: &$s.motion.attributes,
                player_effects: &$s.potions.effects,
                player_uuid,
                player_attachments: &$s.player_state.attachments,
                entity_attachments: &*$s.entity_attachments,
            },
            tag_items: $tags,
        }
    }};
}

/// The world's packs' content files, as a field-level borrow (see the macros).
pub fn pack_files(world_tables: &Option<WorldTables>) -> sunrise_world::loot::PackOverlay<'_> {
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

#[path = "game/modding/api_values.rs"]
pub mod api_values;
#[path = "game/world/autosave.rs"]
pub mod autosave;
#[path = "game/blocks/banners.rs"]
pub mod banners;
#[cfg(test)]
#[path = "game/modding/bare_declaration_names_tests.rs"]
pub mod bare_declaration_names_tests;
#[path = "game/blocks/block_items.rs"]
pub mod block_items;
#[path = "game/modding/block_notifications.rs"]
pub mod block_notifications;
#[path = "game/runtime/blockctl.rs"]
pub mod blockctl;
#[path = "game/interface/books_common.rs"]
pub mod books_common;
#[path = "game/mobs/breezes.rs"]
pub mod breezes;
#[path = "game/blocks/brush.rs"]
pub mod brush;
#[path = "game/items/cake_eligibility.rs"]
pub mod cake_eligibility;
#[path = "game/blocks/campfire_douse.rs"]
pub mod campfire_douse;
#[path = "game/entities/cart_rails.rs"]
pub mod cart_rails;
#[path = "game/blocks/cartography.rs"]
pub mod cartography;
#[path = "game/network/cauldron_prediction.rs"]
pub mod cauldron_prediction;
#[path = "game/blocks/cauldrons.rs"]
pub mod cauldrons;
#[path = "game/interface/chat_common.rs"]
pub mod chat_common;
#[path = "game/world/class_tags.rs"]
pub mod class_tags;
#[cfg(test)]
#[path = "game/world/class_tags_tests.rs"]
pub mod class_tags_tests;
#[path = "game/items/clip_ray.rs"]
pub mod clip_ray;
#[path = "game/world/clocks.rs"]
pub mod clocks;
#[path = "game/entities/closest_space.rs"]
pub mod closest_space;
#[path = "game/entities/collision_source.rs"]
pub mod collision_source;
#[path = "game/administration/command_blocks_common.rs"]
pub mod command_blocks_common;
#[path = "game/interface/command_registries.rs"]
pub mod command_registries;
#[path = "game/administration/commands_common.rs"]
pub mod commands_common;
#[path = "game/world/condition_reload_state.rs"]
pub mod condition_reload_state;
#[path = "game/mobs/copper_golems.rs"]
pub mod copper_golems;
#[path = "game/blocks/machines/crafter_common.rs"]
pub mod crafter_common;
#[path = "game/mobs/creatures/mod.rs"]
pub mod creatures;
#[path = "game/modding/data_map_files.rs"]
pub mod data_map_files;
#[path = "game/modding/data_map_json.rs"]
pub mod data_map_json;
#[path = "game/interface/debug_lines.rs"]
pub mod debug_lines;
#[path = "game/blocks/decorated_pot.rs"]
pub mod decorated_pot;
#[path = "game/interface/developer_launch.rs"]
pub mod developer_launch;
#[path = "game/world/end_common.rs"]
pub mod end_common;
#[path = "game/modding/entity_sections.rs"]
pub mod entity_sections;
#[path = "game/modding/entity_type_rows.rs"]
pub mod entity_type_rows;
#[path = "game/items/equip_answers.rs"]
pub mod equip_answers;
#[path = "game/runtime/execution_role.rs"]
pub mod execution_role;
#[path = "game/world/integrated_common.rs"]
pub mod integrated_common;
#[path = "game/modding/item_snapshot.rs"]
pub mod item_snapshot;
#[path = "game/runtime/memory_common.rs"]
pub mod memory_common;
#[path = "game/modding/native_chunk_snapshot.rs"]
pub mod native_chunk_snapshot;
#[path = "game/modding/native_datapack_values_common.rs"]
pub mod native_datapack_values;
#[path = "game/modding/native_entity_previous_common.rs"]
pub mod native_entity_previous_common;
#[cfg(test)]
#[path = "game/modding/native_living_travel_moved_tests.rs"]
mod native_living_travel_moved_tests;
#[path = "game/modding/native_placement_modifiers.rs"]
pub mod native_placement_modifiers;
#[path = "game/modding/native_rule_tests.rs"]
pub mod native_rule_tests;
#[path = "game/modding/native_structure_processors.rs"]
pub mod native_structure_processors;
#[path = "game/items/stack_names.rs"]
pub mod stack_names;
#[path = "game/world/tables.rs"]
pub mod tables;
#[path = "game/administration/test_registry_tables.rs"]
pub mod test_registry_tables;
#[path = "game/world/world_entry_common.rs"]
pub mod world_entry_common;
#[allow(unused_imports)]
pub use tables::level_suffocation_of;
#[path = "game/modding/api_values_common.rs"]
pub mod api_values_common;
#[path = "game/blocks/beacon_common.rs"]
pub mod beacon_common;
#[path = "game/mobs/beehives_common.rs"]
pub mod beehives_common;
#[path = "game/blocks/block_edits_common.rs"]
pub mod block_edits_common;
#[path = "game/blocks/block_entity_anims_common.rs"]
pub mod block_entity_anims_common;
#[path = "game/blocks/block_props.rs"]
pub mod block_props;
#[path = "game/entities/boats_common.rs"]
pub mod boats_common;
#[path = "game/blocks/bookshelves_common.rs"]
pub mod bookshelves_common;
#[path = "game/mobs/dragon_common.rs"]
pub mod dragon_common;
#[path = "game/entities/entity_common.rs"]
pub mod entity_common;
#[path = "game/modding/entity_initialization_common.rs"]
pub mod entity_initialization_common;
#[path = "game/modding/entity_sizes_common.rs"]
pub mod entity_sizes_common;
#[path = "game/mobs/fangs_common.rs"]
pub mod fangs_common;
#[path = "game/entities/fireworks.rs"]
pub mod fireworks;
#[path = "game/modding/fluid_interactions.rs"]
pub mod fluid_interactions;
#[path = "game/interface/game_mode_common.rs"]
pub mod game_mode_common;
#[path = "game/items/glide_wear.rs"]
pub mod glide_wear;
#[path = "game/modding/global_loot_modifiers.rs"]
pub mod global_loot_modifiers;
#[path = "game/items/held_items.rs"]
pub mod held_items;
#[path = "game/interface/hit.rs"]
pub mod hit;
pub mod host;
#[path = "game/interface/inventory_slots.rs"]
pub mod inventory_slots;
#[path = "game/items/inventory_tick.rs"]
pub mod inventory_tick;
#[path = "game/items/item_abilities.rs"]
pub mod item_abilities;
#[path = "game/items/item_actions.rs"]
pub mod item_actions;
#[path = "game/items/item_entity_spawn.rs"]
pub mod item_entity_spawn;
#[path = "game/items/item_use_stacks.rs"]
pub mod item_use_stacks;
#[path = "game/administration/jigsaw_blocks.rs"]
pub mod jigsaw_blocks;
#[path = "game/blocks/jukebox_common.rs"]
pub mod jukebox_common;
#[path = "game/network/lan_actions/mod.rs"]
pub mod lan_actions;
#[path = "game/runtime/launch_api.rs"]
pub mod launch_api;
#[path = "game/runtime/launch_options.rs"]
pub mod launch_options;
#[path = "game/blocks/level_access.rs"]
pub mod level_access;
#[path = "game/world/light_emission.rs"]
pub mod light_emission;
#[path = "game/modding/load_conditions.rs"]
pub mod load_conditions;
#[path = "game/blocks/location_world.rs"]
pub mod location_world;
#[path = "game/modding/loot_tags.rs"]
pub mod loot_tags;
#[path = "game/modding/loot_types.rs"]
pub mod loot_types;
#[path = "game/interface/menu_clicks.rs"]
pub mod menu_clicks;
#[path = "game/entities/minecarts.rs"]
pub mod minecarts;
#[path = "game/modding/mod_api_host.rs"]
pub mod mod_api_host;
#[path = "game/modding/mod_api_host_common.rs"]
pub mod mod_api_host_common;
#[path = "game/modding/mod_crafting_commands.rs"]
pub mod mod_crafting_commands;
#[path = "game/modding/mods/mod.rs"]
pub mod mods;
#[path = "game/mobs/mounts_common.rs"]
pub mod mounts_common;
#[path = "game/entities/move_gate.rs"]
pub mod move_gate;
#[path = "game/blocks/moving_pistons.rs"]
pub mod moving_pistons;
#[path = "game/modding/native_biome_source.rs"]
pub mod native_biome_source;
#[path = "game/modding/native_block_entity_transform.rs"]
pub mod native_block_entity_transform;
#[path = "game/modding/native_block_extra_models.rs"]
pub mod native_block_extra_models;
#[path = "game/modding/native_block_sounds.rs"]
pub mod native_block_sounds;
#[path = "game/modding/native_block_states.rs"]
pub mod native_block_states;
#[path = "game/modding/native_block_tables.rs"]
pub mod native_block_tables;
#[path = "game/modding/native_book_displays.rs"]
pub mod native_book_displays;
#[path = "game/modding/native_book_placement.rs"]
pub mod native_book_placement;
#[path = "game/modding/native_bots_common.rs"]
pub mod native_bots_common;
#[path = "game/modding/native_cauldron_contents.rs"]
pub mod native_cauldron_contents;
#[path = "game/modding/native_chunk_generator.rs"]
pub mod native_chunk_generator;
#[path = "game/modding/native_collision_exclusions.rs"]
pub mod native_collision_exclusions;
#[path = "game/modding/native_collision_shape.rs"]
pub mod native_collision_shape;
#[path = "game/modding/native_components.rs"]
pub mod native_components;
#[path = "game/modding/native_configs.rs"]
pub mod native_configs;
#[path = "game/modding/native_custom_stats.rs"]
pub mod native_custom_stats;
#[path = "game/modding/native_data_bound.rs"]
pub mod native_data_bound;
#[path = "game/modding/native_datafix.rs"]
pub mod native_datafix;
#[path = "game/modding/native_datapack_refs.rs"]
pub mod native_datapack_refs;
#[path = "game/modding/native_dimensions.rs"]
pub mod native_dimensions;
#[path = "game/modding/native_effect_category.rs"]
pub mod native_effect_category;
#[cfg(test)]
#[path = "game/modding/native_entity_move_tests.rs"]
pub mod native_entity_move_tests;
#[path = "game/modding/native_entity_refs_common.rs"]
pub mod native_entity_refs_common;
#[path = "game/modding/native_entity_selectors.rs"]
pub mod native_entity_selectors;
#[path = "game/modding/native_feature_types.rs"]
pub mod native_feature_types;
#[path = "game/modding/native_game_event_keys.rs"]
pub mod native_game_event_keys;
#[path = "game/modding/native_game_rules_read.rs"]
pub mod native_game_rules_read;
#[path = "game/modding/native_give_stack.rs"]
pub mod native_give_stack;
#[path = "game/modding/native_ground.rs"]
pub mod native_ground;
#[path = "game/modding/native_harvest_queries_common.rs"]
pub mod native_harvest_queries_common;
#[path = "game/modding/native_hit_effects.rs"]
pub mod native_hit_effects;
pub mod native_hooks;
#[path = "game/modding/native_item_bundle.rs"]
pub mod native_item_bundle;
#[path = "game/modding/native_item_container.rs"]
pub mod native_item_container;
#[path = "game/modding/native_item_entity_refs.rs"]
pub mod native_item_entity_refs;
#[path = "game/modding/native_item_menu_parts.rs"]
pub mod native_item_menu_parts;
#[path = "game/modding/native_item_registry.rs"]
pub mod native_item_registry;
#[path = "game/modding/native_knockback_credit.rs"]
pub mod native_knockback_credit;
#[path = "game/modding/native_level_signals.rs"]
pub mod native_level_signals;
#[path = "game/modding/native_lifecycle_common.rs"]
pub mod native_lifecycle_common;
#[path = "game/modding/native_living_absorption.rs"]
pub mod native_living_absorption;
#[path = "game/modding/native_living_blocking.rs"]
pub mod native_living_blocking;
#[path = "game/modding/native_living_body.rs"]
pub mod native_living_body;
#[cfg(test)]
#[path = "game/modding/native_living_travel_tests.rs"]
pub mod native_living_travel_tests;
#[path = "game/modding/native_menu_refs_common.rs"]
pub mod native_menu_refs_common;
#[path = "game/modding/native_motion.rs"]
pub mod native_motion;
#[path = "game/modding/native_namespaced_id.rs"]
pub mod native_namespaced_id;
#[path = "game/modding/native_player_death.rs"]
pub mod native_player_death;
#[path = "game/modding/native_player_food.rs"]
pub mod native_player_food;
#[path = "game/modding/native_player_reach.rs"]
pub mod native_player_reach;
#[path = "game/modding/native_pool_elements.rs"]
pub mod native_pool_elements;
#[path = "game/modding/native_registry_ids.rs"]
pub mod native_registry_ids;
#[path = "game/modding/native_rotate.rs"]
pub mod native_rotate;
#[path = "game/modding/native_server.rs"]
pub mod native_server;
#[path = "game/modding/native_state_shapes.rs"]
pub mod native_state_shapes;
#[path = "game/modding/native_structure_types.rs"]
pub mod native_structure_types;
#[path = "game/modding/native_surface.rs"]
pub mod native_surface;
#[path = "game/modding/native_tags.rs"]
pub mod native_tags;
#[path = "game/modding/native_text_json.rs"]
pub mod native_text_json;
#[path = "game/modding/native_text_legacy.rs"]
pub mod native_text_legacy;
#[path = "game/modding/native_text_nbt.rs"]
pub mod native_text_nbt;
#[path = "game/modding/native_ticks.rs"]
pub mod native_ticks;
#[path = "game/modding/native_trapdoor_ladder.rs"]
pub mod native_trapdoor_ladder;
#[path = "game/modding/native_tree_grow.rs"]
pub mod native_tree_grow;
#[path = "game/modding/native_world_columns.rs"]
pub mod native_world_columns;
#[path = "game/modding/native_worldgen.rs"]
pub mod native_worldgen;
#[path = "game/modding/native_worldgen_parts.rs"]
pub mod native_worldgen_parts;
#[path = "game/mobs/ominous_item_spawner/common.rs"]
pub mod ominous_item_spawner_common;
#[path = "game/modding/pack_finders/common.rs"]
pub mod pack_finders_common;
#[path = "game/modding/part_admission.rs"]
pub mod part_admission;
#[cfg(test)]
#[path = "game/modding/part_admission_tests.rs"]
pub mod part_admission_tests;
#[path = "game/entities/passengers_common.rs"]
pub mod passengers_common;
#[path = "game/modding/payload_transport.rs"]
pub mod payload_transport;
#[path = "game/entities/pick_entity_common.rs"]
pub mod pick_entity_common;
#[path = "game/entities/pick_ray.rs"]
pub mod pick_ray;
#[path = "game/mobs/piglins_common.rs"]
pub mod piglins_common;
#[path = "game/blocks/piston_carry.rs"]
pub mod piston_carry;
#[path = "game/world/piston_nbt.rs"]
pub mod piston_nbt;
#[path = "game/blocks/piston_push.rs"]
pub mod piston_push;
#[path = "game/entities/player_equip.rs"]
pub mod player_equip;
#[path = "game/player_identity.rs"]
pub mod player_identity;
#[path = "game/blocks/potent_sulfur_common.rs"]
pub mod potent_sulfur_common;
#[path = "game/blocks/powder_snow.rs"]
pub mod powder_snow;
#[path = "game/items/primary_items.rs"]
pub mod primary_items;
#[path = "game/items/projectile_spawned_validation.rs"]
pub mod projectile_spawned_validation;
#[path = "game/modding/published_loops.rs"]
pub mod published_loops;
#[path = "game/modding/registration_capability_coverage.rs"]
pub mod registration_capability_coverage;
#[path = "game/network/remote/mod.rs"]
pub mod remote;
#[path = "game/entities/residency_tick.rs"]
pub mod residency_tick;
#[path = "game/entities/resident_state.rs"]
pub mod resident_state;
#[path = "game/runtime/resource_load.rs"]
pub mod resource_load;
#[path = "game/runtime/resource_tables.rs"]
pub mod resource_tables;
#[cfg(any(test, feature = "test-support"))]
#[path = "game/modding/rule_replacements.rs"]
pub mod rule_replacements;
#[path = "game/world/saving.rs"]
pub mod saving;
#[path = "game/modding/scheduled_tasks.rs"]
pub mod scheduled_tasks;
#[path = "game/blocks/shelf_common.rs"]
pub mod shelf_common;
#[path = "game/side_ops/mod.rs"]
pub mod side_ops;
#[path = "game/modding/slot_rules.rs"]
pub mod slot_rules;
#[path = "game/interface/smithing_icons.rs"]
pub mod smithing_icons;
#[path = "game/items/spear.rs"]
pub mod spear;
#[path = "game/modding/stacked_on_other.rs"]
pub mod stacked_on_other;
#[cfg(test)]
#[path = "game/runtime/startup_cpu_tests.rs"]
pub mod startup_cpu_tests;
#[path = "game/interface/startup_options.rs"]
pub mod startup_options;
#[path = "game/runtime/startup_progress.rs"]
pub mod startup_progress;
#[path = "game/modding/state_resolver.rs"]
pub mod state_resolver;
#[path = "game/blocks/stations_common.rs"]
pub mod stations_common;
#[path = "game/world/storage_common.rs"]
pub mod storage_common;
#[path = "game/administration/structures.rs"]
pub mod structures;
#[path = "game/administration/test_entities.rs"]
pub mod test_entities;
#[path = "game/blocks/tool_modification.rs"]
pub mod tool_modification;
#[path = "game/runtime/transactions.rs"]
pub mod transactions;
#[path = "game/modding/typed_payloads.rs"]
pub mod typed_payloads;
#[path = "game/network/villager_client.rs"]
pub mod villager_client;
#[path = "game/blocks/waxing.rs"]
pub mod waxing;
#[path = "game/world/weather.rs"]
pub mod weather;
#[path = "game/world/world_entry.rs"]
pub mod world_entry;
#[path = "game/items/worn_boots.rs"]
pub mod worn_boots;
#[allow(unused_imports)]
pub use self::api_values::TryIntoApi;
pub use api_values::{block_jump_factor, block_speed_factor, is_ladder};
#[allow(unused_imports)]
pub use held_items::interact_location;
pub use hit::Hit;
pub use launch_options::Options;
pub use mod_api::game::DataMapType;
pub use native_hooks::{PhantomSpawnResult, SleepProblem};
#[allow(unused_imports)]
pub use std::{
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
#[allow(unused_imports)]
pub use sunrise_core::{
    biome::{BiomeZoom, obfuscate_seed},
    geometry::{Aabb, Direction},
    position::BlockPos,
};
#[allow(unused_imports)]
pub use sunrise_core::{geometry::Vec3, random::RandomSource};
#[allow(unused_imports)]
pub use sunrise_gameplay_common::lan::idle as lan_idle;
#[allow(unused_imports)]
pub use sunrise_gameplay_common::{interaction_shapes, item_areas, mob_index};
#[allow(unused_imports)]
pub use sunrise_level::random_source;
#[allow(unused_imports)]
pub use sunrise_level::spans::{startup, time as span};
pub use sunrise_level::testgen::TestTerrain;
#[allow(unused_imports)]
pub use sunrise_mod_package::selection::{Argument, LaunchArguments};
pub use sunrise_sim::item_names::Names;
#[allow(unused_imports)]
pub use {
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
#[allow(unused_imports)]
pub use {
    sunrise_game_data::Result, sunrise_items::blocks::BlockFacts, sunrise_items::blocks::Fluid,
    sunrise_items::items::ItemStack, sunrise_items::items::Items, sunrise_level::assets::Assets,
    sunrise_level::biome::Biomes, sunrise_level::collisions,
    sunrise_level::containers::BlockEntity, sunrise_level::containers::Providers,
    sunrise_level::environment::Environment, sunrise_level::light::LightTables,
    sunrise_level::spans, sunrise_level::world::World, sunrise_sim::clock::FixedStep,
    sunrise_sim::crafting::Recipes, sunrise_sim::entities::Experience,
    sunrise_sim::entities::ExperienceOrb, sunrise_sim::entities::ItemEntity,
    sunrise_sim::first_person_sim::Hand, sunrise_sim::first_person_sim::SwingAnimation,
    sunrise_sim::furnace::FurnaceCooking, sunrise_sim::mobs::Mob, sunrise_sim::mobs::MobType,
    sunrise_sim::mobs::MobTypes, sunrise_sim::motion::MotionResources,
    sunrise_sim::motion::PlayerMotion, sunrise_sim::save::Level, sunrise_sim::save::Persistent,
    sunrise_sim::save::SaveDir, sunrise_sim::sprint::Keys, sunrise_sim::vanillagen::VanillaTerrain,
};

/// A frame's finished meshes up to this many upload at once, unqueued.
pub const AUTO_UPLOADS_AT_ONCE: usize = 8;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
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
    pub fn id(self) -> &'static str {
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
pub fn vanilla_tool_state(
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

pub struct Mining {
    pub pos: [i32; 3],
    pub progress: f32,
    /// The latest destroy-speed query, reused for server crack feedback.
    pub per_tick: f32,
    pub ticks: u32,
}
impl Mining {
    pub fn stage(&self) -> Option<u8> {
        (self.progress > 0.0).then(|| ((self.progress * 10.0) as u8).min(9))
    }
}
/// What a transform does beyond the swap: its `transform_type` and the `drop_strategy` its loot table's stacks pop with (both empty for a tool ability outside the table).
#[derive(Clone, Debug, Default)]
pub struct TransformKind {
    pub transform_type: String,
    pub drop_strategy: String,
}

#[derive(Clone, Copy)]
pub struct TransformContext<'a> {
    pub world: &'a World,
    pub player: Option<u32>,
    pub hand: mod_api::Hand,
    pub blocking: bool,
    pub simulated: bool,
}

/// What a falling block's tick did (`FallingBlockEntity.tick`).
pub enum FallStep {
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
pub enum GuestUse {
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
    pub fn on_block(hand: Hand) -> Self {
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
pub struct InteractTarget {
    pub id: i32,
    pub location: [f64; 3],
}

/// The client world as seen by the vanilla block-behaviour library.
pub struct GameLevel<'w> {
    pub cake_probe: bool,
    pub cake_requested: std::cell::Cell<bool>,
    pub cake_answer: Option<bool>,
    pub api_level: Option<mod_api::LevelRef>,
    pub part_state_index: Option<
        &'w std::sync::Arc<
            std::sync::Mutex<
                sunrise_mod_runtime::attachments::index::AttachmentIndex<mod_api::AttachmentHolder>,
            >,
        >,
    >,
    pub attachment_query_scopes: &'w sunrise_mod_runtime::attachments::AttachmentQueryScopes,
    pub positions: piston_carry_common::PositionState<'w>,
    pub border: &'w sunrise_core::geometry::border::BorderGeometry,
    pub world: &'w mut World,
    pub command_block_transition:
        Option<sunrise_gameplay_common::command_blocks::CommandBlockTransition>,
    pub redstone_experiments: bool,
    pub facts: &'w BlockFacts,
    /// The block-state facts a part reads through `LevelHost::block_states`.
    pub tables: native_block_tables::BlockTables<'w>,
    pub collision: &'w BlockCollisionRegistry<'w>,
    pub events: &'w mut Vec<LevelEvent>,
    pub dimension: LevelDimension,
    pub darken: i32,
    /// The environment of the level's dimension (`SUN_ANGLE`).
    pub environment: &'w Environment,
    pub raining: bool,
    pub thundering: bool,
    /// The world's enabled feature flags as this side knows them (`LevelReader.enabledFeatures`).
    pub enabled_features: &'w std::sync::Arc<[String]>,
    pub player_box: Option<([f64; 3], [f64; 3])>,
    /// The LAN guests in this level who trigger blocks (not spectators).
    pub guest_boxes: Vec<([f64; 3], [f64; 3])>,
    /// Other players that block placement during a guest turn, including live
    /// ancestor bodies; None outside one, where `guest_boxes` are those players.
    pub building_player_boxes: Option<Vec<([f64; 3], [f64; 3])>>,
    pub player_collision: Option<PlayerCollision>,
    /// Minecart boxes (detector rails).
    pub cart_boxes: Vec<([f64; 3], [f64; 3])>,
    /// The item frames a comparator reads through a conductor.
    pub item_frames: &'w [sunrise_sim::entities::ItemFrame],
    /// The minecarts, in `cart_boxes` order, for a detector rail's comparator.
    pub carts: &'w mut sunrise_sim::join_seq::JoinList<minecarts::Minecart, false>,
    /// Arrow and thrown trident boxes (wooden buttons).
    pub arrow_boxes: Vec<([f64; 3], [f64; 3])>,
    /// Every entity on the walk; see `entity::for_each_entity`.
    pub other_entities: entity::EntityLists<'w>,
    /// End crystals, which aren't on the walk yet.
    pub crystals: &'w [dragon_common::Crystal],
    /// The fishing hooks here, which aren't on the walk either. None ignores
    /// block triggers, so each counts on a plate or tripwire. The flag says
    /// whether it also reaches `entityInside` (`applyEffectsFromBlocks`): not
    /// a hook on an entity, whose tick skips that step.
    pub trigger_hooks: Vec<(Aabb, bool)>,
    /// The lightning bolts here: none ignores block triggers
    /// (`Entity.isIgnoringBlockTriggers`).
    pub trigger_extras: Vec<Aabb>,
    pub biomes: &'w Biomes,
    pub biome_keys: Option<&'w mod_api::__private::BiomeKeys>,
    /// `BiomeManager`'s zoom over this level's seed, for `biome_name`.
    pub biome_zoom: sunrise_core::biome::BiomeZoom,
    pub seed: i64,
    pub behaviours: &'w Behaviours,
    pub items: &'w Items,
    /// Cooking recipes' experience, which a removed furnace pays out.
    pub recipes: &'w sunrise_sim::crafting::Recipes,
    /// Container loot tables (`unpackLootTable`).
    pub loot: Option<std::sync::Arc<sunrise_world::loot::LootData>>,
    pub loot_storage: std::sync::Arc<sunrise_world::loot::copy::StorageData>,
    /// The level's world clocks, for `time_check` in loot conditions.
    pub loot_clocks: std::collections::BTreeMap<String, i64>,
    pub registry: &'static sunrise_world::registry::BlockRegistry,
    pub block_registries: &'w sunrise_command::args::Registries,
    pub rules: sunrise_world::level::GameRules,
    /// `Difficulty.getId()` (peaceful 0 .. hard 3).
    pub difficulty: i32,
    /// The feet of the players who keep fire spreading around them: the local
    /// player and the LAN guests in this level, spectators aside.
    pub fire_players: Vec<[f64; 3]>,
    /// Block loot random sequences (`ServerLevel.getRandomSequence`).
    pub rand_sequences: &'w mut sunrise_items::random_sequences::RandomSequences,
    pub pending: &'w mut spill_common::Pending,
    pub spill: &'w mut spill_common::Spill,
    /// Where the chunk tickets let blocks tick.
    pub tickets: sunrise_sim::ticket_levels::Levels,
    pub random_tick_sections: &'w mut sunrise_sim::random_tick_sections::RandomTickSections,
    /// Players with each chest open (a trapped chest's signal), also what a
    /// recount of the openers replaces (`ContainerOpenersCounter.recheckOpeners`).
    pub lid_openers: &'w mut HashMap<(i32, i32, i32), i32>,
    /// The containers players have open at this moment, by position: the local
    /// player's and the guests' (`getEntitiesWithContainerOpen`). A double
    /// chest is in it as both halves.
    pub open_containers: &'w [(i32, i32, i32)],
    /// The chests copper golems have open, a double chest as both halves. They
    /// keep their own count (`chest_openers`), so they stay out of `open_containers`.
    pub golem_chests: &'w [(i32, i32, i32)],
    /// `Level.isClientSide`: a client's copy of the level (a guest of a LAN
    /// world, a window on its server thread, a dedicated server's client).
    pub client_side: bool,
    /// Only the integrated or dedicated server dispatches native observations.
    pub native_authoritative: bool,
    pub native_player: Option<u128>,
    /// The acting player's entity id, beside `native_player`.
    pub native_player_entity: Option<i32>,
    /// Direct liquid part writes may not bypass an open Game transaction.
    pub liquid_transaction_open: bool,
    /// `Level.isDebug`: `setBlock` changes nothing.
    pub debug: bool,
    pub native_sounds: &'w mut Vec<creatures::NativeSound>,
    pub loop_context: &'w sunrise_world::native_load_context::LoopContext,
    /// This side's level data, for the read-only level a block's sound type is asked in.
    pub side: native_surface::GameSide<'w>,
    pub native_particles: &'w mut Vec<sunrise_sim::lan::server_effects::ServerQueuedParticle>,
    pub sound_seeds: Option<&'w sunrise_sim::lan::hub::Hub>,
    pub drop_entity: Option<mod_api::EntityRef>,
    /// `LootContextParams.INTERACTING_ENTITY` of a block-interaction drop: the
    /// player whose click `use_without_item` runs for.
    pub interacting: Option<sunrise_world::loot::LootEntity>,
    /// `LootContextParams.TOOL` of a block-interaction drop, when an item triggered the interaction (`None` for bare-hand `use_without_item`).
    pub interacting_tool: Option<ItemStack>,
    /// What a location predicate of such a drop reads beyond the level's own
    /// fields (`blocks::location_world`).
    pub location: location_world::LocationRefs<'w>,
    /// The block a mod's machine breaks and the tool it breaks it with
    /// (`native_machine_destroy_block`): `Block.dropResources` rolls that
    /// block's loot with the tool instead of with none. Neighbours that pop
    /// off because of the break drop as with no tool, as in vanilla.
    pub drop_tool: Option<((i32, i32, i32), Option<ItemStack>)>,
    /// Only this destroyed block has an entity as its loot `this` parameter.
    /// Neighbour/default/machine drops keep the no-entity context.
    pub drop_breaker_at: Option<BlockPos>,
    /// Where `Block.dropResources` puts the drops instead of popping them,
    /// while a mod's machine breaks a block (`native_machine_destroy_block`).
    pub drop_into: Option<Vec<ItemStack>>,
}
pub struct PlayerCollision {
    pub y: f64,
    pub descending: bool,
    pub fall_distance: f64,
    pub main_hand_item: Arc<str>,
    pub main_hand_count: i32,
    pub leather_boots: bool,
}
/// The boxes of the entities that block building, as obstruction candidates.
pub struct BuildingObstruction(Vec<Aabb>);
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
pub fn server_only_block_use(
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
    pub fn boat_trigger_boxes(&self) -> impl Iterator<Item = Aabb> + '_ {
        self.entities().boats.iter().map(boats_common::Boat::bounds)
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

/// A landing's block game event, emitted after its world write is applied.
pub struct LandingVibration {
    event: &'static str,
    position: BlockPos,
    state: BlockStateId,
    from_entity: bool,
}

/// FallingBlockEntity carrying a block state.
#[derive(Clone, Debug)]
pub struct FallingBlock {
    pub attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    /// `Entity.yRot` and `xRot` (`/tp`, `/data get ... Rotation`).
    pub rotation: [f32; 2],
    /// When it joined its list (`sunrise_sim::join_seq`); set by the list.
    pub join: sunrise_sim::join_seq::JoinSeq,
    /// Its entity id: from the entity counter on the server, the server's on a client.
    pub id: u32,
    pub state: BlockStateId,
    /// `FallingBlockEntity.fall` and `recreateFromPacket` set `blocksBuilding`;
    /// constructing an entity from NBT leaves the default false.
    pub blocks_building: bool,
    /// `DATA_START_POS`: spawn position used to select weighted block models.
    pub start_pos: BlockPos,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    /// `FallingBlockEntity.time`: signed, a save can start it below zero.
    pub age: i32,
    /// `Entity.isNoGravity`.
    pub no_gravity: bool,
    /// `FallingBlockEntity.tick` handles portals after moving, before landing.
    pub portal: sunrise_sim::nether_portal::PortalState,
    /// `FallingBlockEntity.dropItem`.
    pub drop_item: bool,
    /// `Entity.fallDistance` (powder snow holds a block that fell far at 0.9).
    pub fall_distance: f64,
    /// `Entity.onGround`: whether its last move was stopped going down.
    pub ground: sunrise_world::ground_support::GroundState,
    /// `Entity.remainingFireTicks` (`Fire`): lava sets it, and it never counts
    /// down, as `FallingBlockEntity.tick` runs no `baseTick`.
    pub fire_ticks: i32,
    /// `Entity.syncVelocity`, set by `markHurt` (lava's hurt): a server sends
    /// its motion to the clients tracking it after its next update.
    pub sync_velocity: bool,
    /// `Entity.stuckSpeedMultiplier`: a cobweb it is in scales its next move.
    pub stuck: Vec3,
    pub damage: sunrise_sim::falling_block::Damage,
    /// The UUID and compound it was loaded with, kept while it moves.
    pub saved: Option<Box<([i32; 4], sunrise_core::nbt::Tag)>>,
    /// What `entity_size` answered at its last size refresh (`EntityEvent.Size`): its box takes
    /// the answer while it holds for its vanilla size.
    pub size_answer: Option<sunrise_sim::pose::SizeAnswer>,
}

sunrise_sim::impl_joined!(FallingBlock);

impl sunrise_sim::entity_rules::EntityRules for FallingBlock {
    fn is_no_gravity(&self) -> bool {
        self.no_gravity
    }
    /// `FallingBlockEntity.getDefaultGravity`.
    fn get_default_gravity(&self) -> f64 {
        0.04
    }
}

sunrise_mod_runtime::impl_attachment_lifetime!(FallingBlock);

impl FallingBlock {
    /// `EntityType.FALLING_BLOCK.getDimensions()`: width, height and eye height.
    const DIMENSIONS: [f32; 3] = [0.98, 0.98, 0.833_000_06];
    /// Its box (`falling_block::bounds`), or its `entity_size` answer's.
    pub fn bounds(&self) -> sunrise_core::geometry::Aabb {
        let vanilla = sunrise_sim::falling_block::bounds(self.position);
        sunrise_sim::pose::SizeAnswer::sized_box(
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
    pub fn model_origin(
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
pub fn falling_block_visible(state: BlockStateId, cell: BlockStateId) -> bool {
    state != cell
}

/// The data loaded once at startup and never changed: blocks, items,
/// recipes, biomes, mobs, collision and motion. Read-only and `Send + Sync`,
/// so the client and an integrated server thread can share one copy.
pub struct Resources {
    pub core: WorldCore,
    pub rest: std::sync::OnceLock<WorldRest>,
}

pub struct WorldCore {
    pub item_data: Arc<sunrise_world::item_defaults::ItemData>,
    facts: BlockFacts,
    pub biomes: Arc<Biomes>,
    environment: Environment,
    pub collision: Arc<BlockCollisionRegistry<'static>>,
    pub motion: MotionResources,
    suffocation: sunrise_world::suffocation::SuffocationRegistry<'static>,
    pub variants: Arc<sunrise_sim::mobs::variants::VariantTables>,
    threads: usize,
}

pub struct WorldRest {
    items: Arc<Items>,
    recipes: Recipes,
    /// The open world's `block_transformer` registry, published by `Resources::set_transformers`.
    transformers: std::sync::RwLock<Arc<sunrise_sim::block_transform::Transformers>>,
    mob_types: HashMap<String, Arc<MobType>>,
    /// The registry keys of the player, the armor stand and the dragon, resolved with these
    /// resources as each mob type's own key is ([`MobType::type_key`]).
    living_type_keys: sunrise_sim::health::LivingTypeKeys,
    pub mob_data: sunrise_sim::mobs::MobData,
    /// Complete pre-creation facts for the registered entity types.
    pub spawner_types: HashMap<String, sunrise_sim::mobs::spawner::SpawnerTypeFacts>,
    pub providers: Providers,
    nether_environment: Environment,
    end_environment: Environment,
    pub behaviours: Arc<Behaviours>,
    stations: sunrise_sim::stations::StationData,
    pub test_environments: Arc<HashMap<String, crate::test_environments::Environment>>,
    pub test_instances: Arc<HashMap<String, crate::test_instances::TestDef>>,
}

/// The content tables a world's data packs change, loaded when the world
/// opens and on `/reload`: `ReloadableServerResources` loads a server's
/// recipes, loot tables and tags from the world's pack stack, over the
/// resource root's own. Worlds without packs use `Resources.rest` directly.
pub struct WorldTables {
    pub conditions: sunrise_world::conditions::LoadConditions,
    pub suffocation: sunrise_world::suffocation::SuffocationRegistry<'static>,
    /// The pack files the tables were built from (a native mod's among
    /// them), for the advancement tree reload and the loot data.
    pub files: Vec<(String, Vec<Vec<u8>>)>,
    pub priority_layers: Vec<sunrise_world::loot::RecipePriorityLayer>,
    pub items: Arc<Items>,
    /// Registry definitions stay frozen at world open, even when tags reload.
    pub packed_registries: Arc<sunrise_sim::lan::data::PackRegistryDefinitions>,
    pub painting_variants: crate::painting_variants::PaintingVariants,
    /// `block_transformer`, bound to the block registry.
    pub transformers: Arc<sunrise_sim::block_transform::Transformers>,
    /// Frozen `minecraft:timeline` entries and their bound tags, read by `Environment` and `/time`.
    pub timelines: sunrise_level::environment::TimelineTable,
    /// Data-driven mob variant and sound-variant registries, frozen at world open.
    pub variants: Arc<sunrise_sim::mobs::variants::VariantTables>,
    pub providers: Providers,
    pub biomes: Arc<condition_reload_state::WorldBiomeRegistry>,
    pub loot: Arc<sunrise_world::loot::LootData>,
    pub loot_modifier_manager: Arc<mod_api::GlobalLootModifierManager>,
    pub recipes: Recipes,
    pub stations: sunrise_sim::stations::StationData,
    pub reload_sources: Option<Arc<sunrise_sim::crafting::ResolvedSources>>,
    /// The values of the data map types native mods registered (`native_data_map_types`).
    pub data_maps: native_data_maps_common::DataMaps,
    /// Pack tags used by advancement predicates and built-in data maps
    /// (`TagLoader.loadTagsForExistingRegistries`). Damage tags live in the catalog.
    pub pack_tags: HashMap<&'static str, HashMap<String, HashSet<String>>>,
    /// The gameplay attributes of the dimension types, biomes and timelines (with the packs' timeline tags) that the raid code reads.
    pub gameplay: Arc<sunrise_sim::patrol::GameplayAttributes>,
    /// `Registries.TEST_ENVIRONMENT` and `Registries.TEST_INSTANCE`, frozen at world open.
    pub test_environments: Arc<HashMap<String, crate::test_environments::Environment>>,
    pub test_instances: Arc<HashMap<String, crate::test_instances::TestDef>>,
}

pub struct PreparedParts {
    block_states: Arc<api_values::block_state::Index>,
    parts: Option<mod_api::Parts>,
    cauldrons: Arc<cauldron_interactions::Table>,
}
#[allow(unused_imports)]
pub use condition_reload_state::PreparedWorldReload;

impl WorldTables {
    /// The registries [`WorldTables::pack_tags`] holds.
    pub const PACK_TAG_REGISTRIES: [&'static str; 2] = ["worldgen/biome", "worldgen/structure"];
}

/// The equipment slot of `armor[index]`: head, chest, legs, feet.
/// `doHurtEquipment`'s `durabilityDamage` before the cast to int: `max(1, damage / 4)`.
pub fn durability_damage(damage: f32) -> f32 {
    (damage / 4.0).max(1.0)
}
/// The durability damage a mod left for `slot` (`mod_api::ArmorHurt::new_damage`, cast to int
/// as vanilla casts it); an entry it removed keeps vanilla's `fallback`.
pub fn answered_wear(
    entries: &[mod_api::ArmorHurt],
    slot: sunrise_core::equipment_slot::EquipmentSlot,
    fallback: u32,
) -> u32 {
    entries
        .iter()
        .find(|e| e.slot == slot)
        .map_or(fallback, |e| e.new_damage as u32)
}

pub fn armor_slot(index: usize) -> sunrise_core::equipment_slot::EquipmentSlot {
    use sunrise_core::equipment_slot::EquipmentSlot;
    match index {
        0 => EquipmentSlot::Head,
        1 => EquipmentSlot::Chest,
        2 => EquipmentSlot::Legs,
        _ => EquipmentSlot::Feet,
    }
}

/// What a resource pack reload replaces (`Minecraft.reloadResourcePacks`):
/// block models and sprites with their meshing tables, item models,
/// translations and the textures read on the CPU. Each game holds its own;
/// cloning shares the loaded data.
#[derive(Clone)]
pub struct PackResources {
    pub source: sunrise_assets::resource_source::ResourceSource,
    pub data_source: sunrise_assets::resource_source::ResourceSource,
    pub data_changes: sunrise_assets::resource_source::ResourceSource,
    pub additional_data: Arc<[Arc<sunrise_level::datapack::DataPack>]>,
    pub mod_layers: Arc<[sunrise_assets::resource_source::ResourceSource]>,
    pub assets: Arc<Assets>,
    pub block_registries: Arc<sunrise_command::args::Registries>,
    pub names: Arc<Names>,
    pub chunk_context: Arc<sunrise_level::chunk_streamer::ChunkWorkerContext>,
    pub item_models: Arc<sunrise_assets::item_model::ItemModels>,
    pub item_ids: Arc<HashMap<String, i32>>,
}

/// A quick craft drag as the screen draws it: each dragged slot's count and
/// whether the slot caps it (None: hidden), and what stays in hand.
pub struct DragView {
    pub slots: HashMap<usize, Option<(u32, bool)>>,
    pub left: Option<u32>,
}
pub fn equipment_modifier_applies(group: &str, slot: &str) -> bool {
    group == slot
        || group == "any"
        || (group == "armor" && slot != "mainhand" && slot != "offhand")
        || (group == "hand" && (slot == "mainhand" || slot == "offhand"))
}
/// Ctrl+Q reads the physical keyboard, independently of sprint key mappings.
/// A chunk whose ticking future can complete (`ChunkMap.prepareTickingChunk`):
/// lit, with its neighbours loaded, as `postprocess_chunks` waits for.
pub fn ticking_chunk(world: &World, x: i32, z: i32) -> bool {
    world.chunks.is_ticking_ready((x, z))
}
/// `TicketStorage.canTicketExpire` for a `PORTAL` ticket: its chunk's holder is
/// ready for saving once the futures of its level are done, the chunk lit and
/// its neighbours loaded out to one chunk at a ticking level
/// (`prepareTickingChunk`) and two at an entity-ticking one
/// (`prepareEntityTickingChunk`). The ticket's own level stands for the
/// holder's: the area it waits for is the one its own streaming loads.
pub fn portal_ticket_can_expire(
    world: &World,
    ticket: &sunrise_sim::ticket_levels::PortalTicket,
) -> bool {
    use sunrise_sim::ticket_levels::{BLOCK_TICKING, ENTITY_TICKING};
    let r = match ticket.level {
        l if l <= i32::from(ENTITY_TICKING) => 2,
        l if l == i32::from(BLOCK_TICKING) => 1,
        _ => 0,
    };
    let (x, z) = ticket.chunk;
    world.chunks.get(&(x, z)).is_some_and(|c| c.lit)
        && (-r..=r).all(|dz| (-r..=r).all(|dx| world.chunks.contains_key(&(x + dx, z + dz))))
}
/// The icon the Select World list shows for the world in play
/// (`GameRenderer.tryTakeScreenshotIfNeeded`): its path in the save, and when
/// the last frame grab was asked for.
pub struct WorldIcon {
    pub path: PathBuf,
    pub last_attempt: Option<Instant>,
}

#[derive(Debug, PartialEq)]
pub struct QueuedLevelEvent {
    pub kind: i32,
    pub pos: [i32; 3],
    pub data: i32,
    pub source: Option<sunrise_world::level::LevelEventPlayer>,
}

/// Temporary block-contact collections rebuilt on every server tick. Keeping
/// their backing storage on `Game` avoids reallocating them while the world is
/// ticking; the wrapper around `tick_blocks` restores the scratch even when a
/// phase returns an error.
#[derive(Default)]
pub struct TickScratch {
    pub ticking: Vec<(i32, i32)>,
    pub inside: Vec<BlockPos>,
    pub standing: Vec<(BlockPos, bool, f64)>,
    pub boat_cells: Vec<BlockPos>,
    pub touched: Vec<BlockPos>,
    pub sensing: Vec<BlockPos>,
    pub rest: Vec<BlockPos>,
    pub later: Vec<BlockPos>,
}

#[allow(dead_code)]
pub trait Side:
    Sized
    + crate::game::side_ops::blocks::SideOps
    + crate::game::side_ops::events::SideOps
    + crate::game::side_ops::items::SideOps
    + crate::game::side_ops::lan::SideOps
    + crate::game::side_ops::mobs_creatures::SideOps
    + crate::game::side_ops::mobs_rest::SideOps
    + crate::game::side_ops::modapi::SideOps
    + crate::game::side_ops::modapi_hooks::SideOps
    + crate::game::side_ops::modapi_modding::SideOps
    + crate::game::side_ops::player::SideOps
    + crate::game::side_ops::presentation::SideOps
    + crate::game::side_ops::tables::SideOps
    + crate::game::side_ops::ui::SideOps
    + crate::game::side_ops::world::SideOps
{
    type Nominal<'g>;

    const IS_SERVER: bool;
    fn streaming_state(&self) -> &sunrise_level::chunk_streamer::ChunkState;
    fn shulkers(
        &self,
    ) -> &sunrise_level::fast_hash::FxHashMap<[i32; 3], sunrise_level::shulker_box::Shulker>;
    fn auto_jump(&self) -> bool;
    fn main_hand_left(&self) -> bool;
    fn operator_items_tab(&self) -> bool;
    fn mods_list(&self) -> &sunrise_mod_package::mod_list::ModList;
    fn mods_list_mut(&mut self) -> &mut sunrise_mod_package::mod_list::ModList;
    fn host_side<'a, 'g>(game: &'g mut Self::Nominal<'a>) -> mod_api::HostSide<'g>;
    fn as_host<'a, 'g>(game: &'g Self::Nominal<'a>) -> &'g dyn mod_api::Host;
    fn as_block_states_backend<'a, 'g>(
        game: &'g Self::Nominal<'a>,
    ) -> &'g dyn mod_api::prelude::BlockStatesBackend;
    fn as_config_context<'a, 'g>(game: &'g Self::Nominal<'a>) -> &'g dyn mod_api::ConfigContext;
    fn as_reload_context<'a, 'g>(game: &'g Self::Nominal<'a>) -> &'g dyn mod_api::ReloadContext;
    fn as_pack_discovery_context_mut<'a, 'g>(
        game: &'g mut Self::Nominal<'a>,
    ) -> &'g mut dyn mod_api::PackDiscoveryContext;
    fn as_server_host_mut<'a, 'g>(
        _game: &'g mut Self::Nominal<'a>,
    ) -> Option<&'g mut dyn mod_api::ServerHost> {
        None
    }
    fn as_client_host_mut<'a, 'g>(
        _game: &'g mut Self::Nominal<'a>,
    ) -> Option<&'g mut dyn mod_api::ClientHost> {
        None
    }
    fn as_client_text_host<'a, 'g>(
        _game: &'g Self::Nominal<'a>,
    ) -> Option<&'g dyn mod_api::prelude::ClientTextClientHost> {
        None
    }
    fn attachment_store<'a, 'g>(
        game: &'g mut Self::Nominal<'a>,
    ) -> &'g mut dyn mod_api::AttachmentStore;
    fn world_tables(&self) -> Option<&WorldTables> {
        None
    }
    fn lan_acting_uuid(&self) -> Option<u128> {
        None
    }
    fn levels_generation(&self, _id: &str) -> u32 {
        0
    }
    fn session_lock_active(&self) -> bool {
        false
    }
    fn level_ref(
        &self,
        _dim: Dim,
        _epoch: Option<mod_api::WorldEpoch>,
    ) -> Option<mod_api::LevelRef> {
        None
    }
    fn held_out(&self) -> &[u32] {
        &[]
    }
    fn lan_acting(&self) -> Option<u32> {
        None
    }
    fn lan_acting_entity(&self) -> Option<u32> {
        None
    }
    fn lan_acting_fake(&self) -> bool {
        false
    }
    fn lan_guest_swaps(&self) -> &[u32] {
        &[]
    }
    fn acting_abilities(&self) -> Option<sunrise_sim::lan::abilities::PlayerAbilities> {
        None
    }
    fn native_command_filter(&self) -> bool {
        false
    }
    fn riding_order(&self) -> i64 {
        0
    }
    fn lan_acting_name(&self) -> Option<&str> {
        None
    }
    fn server_tick_deadline(&self) -> Option<Instant> {
        None
    }
}

pub struct CommonGame<'a> {
    /// Shared with an integrated server; `collision` and `motion` borrow from it.
    pub res: Arc<Resources>,
    pub rest_loader: Option<WorldRestLoader>,
    pub collision: &'a BlockCollisionRegistry<'a>,
    pub motion: &'a MotionResources,
    /// This world's frozen damage definitions and reloadable tag snapshot.
    pub health_resources: sunrise_sim::health::HealthResources,
    pub damage_tag_generation: u64,
    pub pack: PackResources,
    pub world: World,
    /// The biome table the world plays with: the game's own with the world's
    /// packs over it (`Biomes::with_packs`), `None` before a world opens.
    pub world_biomes: Option<Arc<Biomes>>,
    /// `world_biomes` as the biome files made it, before `native_biome_colors`.
    pub world_biomes_base: Option<Arc<Biomes>>,
    pub options: Options,
    pub player: Option<PlayerMotion>,
    pub spawn: Option<Vec3>,
    pub clock: FixedStep,
    pub previous_position: Vec3,
    pub eye: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub inventory: Vec<Option<ItemStack>>,
    pub selected: usize,
    pub carried: Option<ItemStack>,
    pub craft_grid: Vec<Option<ItemStack>>,
    pub craft_size: usize,
    /// Open container block entity, if any.
    pub container: Option<(i32, i32, i32)>,
    /// Open workstation menu (enchanting table, anvil, ...).
    pub station: Option<stations_common::Station>,
    /// Status effects and thrown potions (game/items/potions.rs).
    pub potions: potions_common::PotionState,
    pub scratch_slot: Option<ItemStack>,
    pub mining: Option<Mining>,
    pub use_delay: u32,
    pub creative: bool,
    /// The game mode; `creative` mirrors `mode == Creative`.
    pub mode: sunrise_command::args::GameMode,
    /// Minecraft.pause: a single-player screen that pauses the game is open.
    pub pause: bool,
    pub previous: Instant,
    pub ticks: u64,
    /// What ends the game with an error; kept as it is, so main can tell a debug crash.
    pub error: Option<Box<dyn std::error::Error>>,
    pub level: Level,
    /// The side, registries and mod settings of this game's resource generation, which
    /// its menus lend to the hooks they fire inside a loop; rebuilt by `refresh_loop_context`.
    pub loop_context: Arc<sunrise_world::native_load_context::LoopContext>,
    /// This thread's authority over the game's tick-owned state (`parallel::TickThread`): a
    /// `Game` never leaves the thread that built it.
    pub tick: sunrise_sim::parallel::TickThread,
    /// Authoritative store for this game side's native mod settings registered with `SettingsRegistrar::register_setting`.
    pub native_settings: native_hooks::mod_settings::SharedSettings,
    /// The native damage hooks every level of this game lends to its hits.
    pub damage_hooks: sunrise_sim::health::DamageHooks,
    pub dropped: sunrise_sim::join_seq::JoinList<ItemEntity>,
    pub mobs: sunrise_sim::join_seq::JoinList<Mob>,
    /// `mobs` by network id, for the packets that name one (`game/mobs/mob_index.rs`).
    pub mob_index: mob_index::MobIndex,
    pub creatures: creatures::Creatures,
    /// Item being used (eaten): hotbar slot and ticks used.
    pub using: Option<(usize, u32)>,
    /// `getUseDuration` at the use's start, as a mod left it (`game/items/item_use.rs`).
    pub use_duration: i32,
    /// What that use began with (`useItem`) and whether a mod set its ticks left
    /// (`game/items/item_use.rs`).
    pub use_start: item_use_common::UseStart,
    /// Armor slots: head, chest, legs, feet.
    pub armor: [Option<ItemStack>; 4],
    /// The off hand (Inventory slot 40, saved as slot 150; menu slot 504 here).
    pub offhand: Option<ItemStack>,
    /// The body and saddle slots (`EquipmentSlot.BODY`, `SADDLE`; Inventory
    /// slots 41 and 42, saved as 151 and 152). The hands and armor stay in
    /// the fields above. Nothing in the game reads these yet; they are kept
    /// for mods and saves.
    pub equipment: sunrise_items::equipment::Equipment,
    /// The hand an item use acts with (`InteractionHand`): the off hand only
    /// for its turn in `use_item` and while its item is being used.
    pub hand: Hand,
    /// `Player.getCooldowns()`: the `use_cooldown` component's entries, what
    /// `itemDecorations` shades a slot by. A server's players have their own
    /// (`GuestPlay::cooldowns`), swapped in for their turn.
    pub cooldowns: sunrise_items::cooldowns::Cooldowns,
    /// This game's world is autosaving this tick (`MinecraftServer.isSaving`).
    pub autosaving: bool,
    pub experience: Experience,
    /// `Player.DATA_SCORE_ID`, independent of `totalExperience`.
    pub player_score: i32,
    pub orbs: sunrise_sim::join_seq::JoinList<ExperienceOrb>,
    /// The arm swing (`LivingEntity.SwingState`) and how many swings started.
    pub swing: sunrise_sim::first_person_sim::SwingState,
    pub swings_started: u32,
    /// `Minecraft`'s attack handling, attack clicks since the last tick and
    /// the attack strength tickers the hand reads.
    pub clicks: sunrise_sim::first_person_sim::Clicks<Option<ItemStack>>,
    pub flash: u32,
    /// The head of the bed while sleeping; the visual timer also survives waking.
    pub sleeping: Option<[i32; 3]>,
    pub sleep_timer: i32,
    pub dim: Dim,
    pub server_items: Option<Arc<Items>>,
    /// The world is superflat (`ServerLevel.isFlat`, for every dimension).
    pub world_flat: bool,
    pub portal_player: sunrise_sim::nether_portal::PortalState,
    pub portal_sections: sunrise_sim::portal_sections::PortalSections,
    pub end: end_state::EndState,
    pub block_ticks: Ticks,
    pub level_events: Vec<LevelEvent>,
    pub falling: sunrise_sim::join_seq::JoinList<FallingBlock, false>,
    pub paintings: sunrise_sim::join_seq::JoinList<sunrise_sim::entities::Painting>,
    pub painting_variants: sunrise_gameplay_common::painting_variants::PaintingVariants,
    pub item_frames: sunrise_sim::join_seq::JoinList<sunrise_sim::entities::ItemFrame>,
    pub armor_stands: sunrise_sim::join_seq::JoinList<sunrise_sim::entities::ArmorStand>,
    /// `Cushion` decoration entities (game/entities/cushions.rs).
    pub cushions: sunrise_sim::join_seq::JoinList<sunrise_sim::entities::Cushion>,
    pub boats: sunrise_sim::join_seq::JoinList<boats_common::Boat, false>,
    pub carts: sunrise_sim::join_seq::JoinList<minecarts::Minecart, false>,
    pub bolts: sunrise_sim::join_seq::ResidentList<lightning_common::Bolt>,
    pub maps: maps::Maps,
    pub dragon: dragon_parts_table::DragonSlot,
    pub crystals: sunrise_sim::join_seq::JoinList<dragon_common::Crystal>,
    pub breath_clouds: sunrise_sim::join_seq::ResidentList<dragon_common::Breath>,
    /// The boat the player rides.
    pub riding: Option<u32>,
    /// The mount the last `position_rider` put the player on, to tell the tick
    /// a seat is taken (`Entity.moveTo`, no camera sweep) from the ones that
    /// follow it (`positionRider`, interpolated as usual).
    pub posed_rider: Option<u32>,
    /// Block entity data of broken blocks waiting for their dropped item.
    pub pending: spill_common::Pending,
    /// The player's fishing hook.
    pub hook: sunrise_sim::join_seq::ResidentSlot<fishing_common::Hook>,
    pub tridents: sunrise_sim::join_seq::JoinList<sunrise_sim::thrown_trident::ThrownTrident>,
    pub rockets: sunrise_sim::join_seq::JoinList<fireworks::Rocket>,
    /// Ominous trial spawners' item spawners.
    pub ominous_spawners:
        sunrise_sim::join_seq::ResidentList<ominous_item_spawner_common::OminousItemSpawner>,
    pub tnts: sunrise_sim::join_seq::JoinList<tnt_common::PrimedTnt>,
    /// The open `host.transaction`s, innermost last (`transactions.rs`).
    pub transactions: transactions::OpenTransactions,
    /// Block-entity positions retained between ticks for client effects.
    pub spawner_tick_positions: Vec<sunrise_level::block_entities::Pos>,
    pub trial_spawner_tick_positions: Vec<sunrise_level::block_entities::Pos>,
    /// The `[[native.loop]]` slots' state, per published loop (`published_loops.rs`).
    loop_states: published_loops::LoopStates,
    pub item_features: sunrise_items::creative_catalog::ItemFeatureRequirements,
    pub creative_contents: native_hooks::CreativeCache,
    pub entity_initialization: entity_initialization_common::EntityInitialization,
    pub kinetic_feedback: HashMap<i32, i64>,
    pub control: Option<sunrise_sim::control::Server>,
    /// Sunrise mods (`game/modding/mods/mod.rs`), none running unless turned on.
    pub mods: mods::ModsState,
    /// Every item and recipe, for mods (`game/modding/native_recipes.rs`), made once.
    pub recipe_tables: native_recipe_tables::RecipeTables,
    /// The native lifecycle bookkeeping.
    pub native_lifecycle: native_lifecycle_common::NativeLifecycle,
    pub resident_entities: resident_state::ResidentEntities,
    /// The game's `config/` folder beside `options.txt` (or `server.properties` on a dedicated server), lent by `Host::config_dir`.
    pub config_root: std::sync::Arc<std::path::Path>,
    /// Native mods' config files this game loaded (game/modding/native_configs.rs).
    pub native_configs: native_configs::NativeConfigs,
    /// Private custom-book connection and placement state.
    pub native_net: native_net_common::NativeNet,
    /// What happened, for `events` and `wait-for` (game/runtime/events.rs).
    pub events: events_common::Events,
    /// The overworld's last `sky_darken` and its inputs (time, rain, thunder).
    sky_darken_memo: std::cell::Cell<Option<((Dim, i64, u32, u32), i32)>>,
    pub persist: entity_memory::EntityMemory,
    pub entity_attachments: native_collider_state::EntityMaps,
    pub collider_placements: native_collider_state::Placements,
    /// Typed per-level membership follows dimensions, not the temporary world swap.
    pub attachment_indexes: attachment_indexes::Indexes,
    /// The passages mods registered in each level, and a client's copy (`native_passages.rs`).
    pub passages: native_passages::Passages,
    /// MultiPlayerGameMode.previousLocalPlayerMode (F3+N goes back to it).
    pub previous_mode: Option<sunrise_command::args::GameMode>,
    /// `Player.entityInteractionRange` and `blockInteractionRange`, refreshed
    /// each tick from the attribute (a server's reach modifier, creative's
    /// +2): the pick helpers' `&self` reach.
    pub entity_reach: f64,
    pub block_reach: f64,
    /// Live attachments and original snapshots retained during player replacement.
    pub player_state: player_state::PlayerState,
    pub ui: chat_common::WorldCommandState,
    pub mount: mounts_common::MountState,
    pub stats: sunrise_gameplay_common::stats::Stats,
    pub score: scoreboard_common::ScoreState,
    pub border: sunrise_level::worldborder::WorldBorder,
    pub adv: advancements_common::AdvState,
    pub fangs: fangs_common::FangState,
}
/// The open world's dimensions past vanilla's three. One per process, as a window
/// and its server thread name the same dimensions.
pub static EXTRA_DIMS: std::sync::RwLock<Vec<mod_api::DimensionKey>> =
    std::sync::RwLock::new(Vec::new());
static EXTRA_LEVELS: std::sync::RwLock<Vec<(mod_api::DimensionKey, LevelDimension)>> =
    std::sync::RwLock::new(Vec::new());

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dim {
    Overworld,
    Nether,
    End,
    /// A dimension past vanilla's three, from a data pack or a mod.
    Extra(mod_api::DimensionKey),
}
impl Dim {
    /// The level the world has for `level`; None for a `Custom` level it lacks.
    pub fn from_level(level: LevelDimension) -> Option<Self> {
        Some(match level {
            LevelDimension::Overworld => Self::Overworld,
            LevelDimension::Nether => Self::Nether,
            LevelDimension::End => Self::End,
            LevelDimension::Custom(key) => {
                Self::exists(key.id()).then(|| Self::from_id(key.id()))?
            }
        })
    }

    /// Drops the dimension `id` that [`Self::add_extra`] added: its level is closed, or a remote
    /// server's level that its client left.
    pub fn remove_extra(id: &str) {
        EXTRA_DIMS
            .write()
            .unwrap()
            .retain(|key| mod_api::__private::dimension_id(*key) != Some(id));
        EXTRA_LEVELS
            .write()
            .unwrap()
            .retain(|(key, _)| mod_api::__private::dimension_id(*key) != Some(id));
    }

    /// The level `level`'s world spawn is in (MinecraftServer.findRespawnDimension), the
    /// Overworld when the world has no such level. It borrows only the level data, so a
    /// `TickCtx` can name it beside the world it borrows mutably.
    pub fn of_world_spawn(level: &sunrise_sim::save::Level) -> Self {
        level
            .world_spawn_dimension
            .as_deref()
            .map_or(Self::Overworld, Self::from_id)
    }

    /// The level as block behaviour and the mods name it.
    pub fn level(self) -> LevelDimension {
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
    pub fn id(self) -> &'static str {
        match self {
            Self::Overworld => "minecraft:overworld",
            Self::Nether => "minecraft:the_nether",
            Self::End => "minecraft:the_end",
            Self::Extra(key) => {
                mod_api::__private::dimension_id(key).expect("the game's table made the key")
            }
        }
    }
    pub fn from_id(id: &str) -> Self {
        match id {
            "minecraft:the_nether" => Self::Nether,
            "minecraft:the_end" => Self::End,
            _ => Self::extra(id).unwrap_or(Self::Overworld),
        }
    }
    /// `Level.getSeaLevel` of the level: the sea level its noise settings give a mod's level, else
    /// the built-in level's.
    pub fn sea_level(self) -> i32 {
        Self::sea_level_of_id(self.id())
    }
    /// [`Self::sea_level`] of the level `id` names.
    pub fn sea_level_of_id(id: &str) -> i32 {
        sunrise_core::LevelKey::parse(id).map_or(63, sunrise_world::level::sea_level_of)
    }
    /// The open world's dimension `id` past vanilla's three.
    pub fn extra(id: &str) -> Option<Self> {
        let extra = EXTRA_DIMS.read().unwrap();
        let key = extra
            .iter()
            .find(|key| mod_api::__private::dimension_id(**key) == Some(id))?;
        Some(Self::Extra(*key))
    }
    pub fn is_vanilla(id: &str) -> bool {
        matches!(
            id,
            "minecraft:overworld" | "minecraft:the_nether" | "minecraft:the_end"
        )
    }
    /// Whether this is one of vanilla's built-in levels (Overworld, Nether, End).
    pub fn is_builtin(self) -> bool {
        matches!(self, Self::Overworld | Self::Nether | Self::End)
    }
    /// Whether the open world has the dimension `id`.
    pub fn exists(id: &str) -> bool {
        Self::is_vanilla(id) || Self::extra(id).is_some()
    }
    /// The open world's dimensions, in the order its server ticks them.
    pub fn all() -> Vec<Self> {
        let extra = EXTRA_DIMS.read().unwrap();
        [Self::Overworld, Self::Nether, Self::End]
            .into_iter()
            .chain(extra.iter().copied().map(Self::Extra))
            .collect()
    }
    /// The dimensions past vanilla's three of the world being opened.
    pub fn set_extra<'a>(ids: impl Iterator<Item = &'a str>) {
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
    pub fn add_extra(id: &str) {
        if !Self::exists(id) {
            let key = mod_api::__private::dimension_key(id);
            let level = LevelDimension::of_level(
                sunrise_core::LevelKey::parse(id).unwrap_or(sunrise_core::LevelKey::OVERWORLD),
            );
            EXTRA_DIMS.write().unwrap().push(key);
            EXTRA_LEVELS.write().unwrap().push((key, level));
        }
    }
    /// The dimension as mods' level handles name it.
    pub fn key(self) -> mod_api::DimensionKey {
        match self {
            Self::Overworld => mod_api::DimensionKey::OVERWORLD,
            Self::Nether => mod_api::DimensionKey::NETHER,
            Self::End => mod_api::DimensionKey::END,
            Self::Extra(key) => key,
        }
    }
    /// The level's shape in a world of vanilla's dimension types.
    pub fn shape(self) -> sunrise_sim::save::LevelShape {
        use sunrise_sim::save::LevelShape;
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
pub fn inventory_add(
    inventory: &mut [Option<ItemStack>],
    offhand: &mut Option<ItemStack>,
    selected: usize,
    max: u32,
    stack: &mut ItemStack,
) -> bool {
    use sunrise_sim::menu::{OFFHAND, free_slot, slot_with_remaining_space};
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

pub fn facing_offset(facing: &str) -> [i32; 3] {
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
pub fn ray_box(eye: [f64; 3], dir: [f64; 3], reach: f64, b: Aabb) -> Option<f64> {
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
pub fn view_vector(yaw: f32, pitch: f32) -> [f64; 3] {
    let v = sunrise_world::view_direction::view_vector(pitch, yaw);
    [v.x, v.y, v.z]
}
#[cfg(test)]
pub mod view_vector_tests {
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
pub fn camera_rotation(
    camera_type: sunrise_level::camera_math::CameraType,
    yaw: f32,
    pitch: f32,
) -> glam::Quat {
    camera_rotation_rolled(camera_type, yaw, pitch, 0.0)
}

pub fn camera_rotation_rolled(
    camera_type: sunrise_level::camera_math::CameraType,
    yaw: f32,
    pitch: f32,
    roll: f32,
) -> glam::Quat {
    let (view_yaw, view_pitch, view_roll) =
        if camera_type == sunrise_level::camera_math::CameraType::ThirdPersonFront {
            (yaw + 180.0, -pitch, -roll)
        } else {
            (yaw, pitch, roll)
        };
    let q = sunrise_level::camera_math::rotation_with_roll(view_yaw, view_pitch, view_roll);
    glam::Quat::from_xyzw(q.x, q.y, q.z, q.w)
}
/// The world directions of a camera-facing quad's x and y: `PoseStack.rotate` by the camera's
/// orientation, as `FishingHookRenderer`, `DragonFireballRenderer` and
/// `SubmitNodeCollection.submitNameTag` turn theirs. Both stay whole looking straight up or
/// down, where the view vector has no horizontal part.
pub fn billboard_axes(orientation: glam::Quat) -> [glam::DVec3; 2] {
    [glam::Vec3::X, glam::Vec3::Y].map(|axis| (orientation * axis).as_dvec3())
}
#[cfg(test)]
pub mod billboard_axes_tests {
    /// `Camera.setRotation(30, -90)` (looking straight up) on HotSpot 25.0.4.1: its orientation
    /// turns x and y to these, as float bits. glam's quaternion is not JOML's to the last bit.
    #[test]
    fn a_billboard_looking_straight_up_takes_the_cameras_axes() {
        let expected: [[u32; 3]; 2] = [
            [0xbf5d_b3d8, 0x0000_0000, 0xbf00_0002],
            [0x3f00_0002, 0xb180_0001, 0xbf5d_b3d7],
        ];
        let axes = super::billboard_axes(super::camera_rotation(
            sunrise_level::camera_math::CameraType::FirstPerson,
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
impl crate::game::CommonGame<'_> {
    /// `Level.canHaveWeather`, from the level's dimension type (sky light, no
    /// ceiling) and the End, which vanilla names by id. Every rain, thunder
    /// and lightning rule asks this, never the dimension's id.
    fn level_key(&self) -> sunrise_core::LevelKey {
        sunrise_core::LevelKey::builtin_or_overworld(self.dim.id())
    }
    pub fn dimension_type_of(
        &self,
        dim: Dim,
    ) -> &'static sunrise_world::dimension_type::DimensionFacts {
        sunrise_world::dimension_type::DimensionFacts::of_level(
            sunrise_core::LevelKey::parse(dim.id()).unwrap_or(sunrise_core::LevelKey::OVERWORLD),
        )
    }

    pub fn eye_position(&self) -> Option<[f64; 3]> {
        let p = self.player.as_ref()?;
        Some([
            p.position.x,
            p.position.y + f64::from(p.dimensions.eye_height),
            p.position.z,
        ])
    }
    /// `Player.blockInteractionRange`: the block pick's and mining's reach.
    pub fn reach(&self) -> f64 {
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
        shulkers: &sunrise_level::fast_hash::FxHashMap<
            [i32; 3],
            sunrise_level::shulker_box::Shulker,
        >,
    ) -> Option<[f64; 6]> {
        let name = &self.res.facts().get(state).name;
        if matches!(&**name, "minecraft:light" | "minecraft:scaffolding") {
            return (player_context && self.held().is_some_and(|s| *s.item == **name))
                .then_some([0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
        }
        if !name.ends_with("shulker_box") {
            return None;
        }
        let progress = shulkers.get(&pos).map_or(0.0, |s| s.progress);
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
    /// What the window decides for its player's body every tick: the look
    /// (Player.travel steers sprint-swimming by the pitch), Auto Jump (LocalPlayer.sendPosition), the slowdown of an
    /// item in use (LocalPlayer.modifyInput), the helmet's Respiration
    /// (decreaseAirSupply), the wings, leather boots on powder snow
    /// (PowderSnowBlock.canEntityWalkOnPowderSnow), freeze-immune wear
    /// (LivingEntity.canFreeze), the dimension's FAST_LAVA, the sleeping
    /// pose and blindness (Player.isMobilityRestricted, which stops a sprint).
    /// The window's or the guest's own body reads the item in its selected slot
    /// (`getMainHandItem`) for the shapes that depend on it.
    pub fn sync_main_hand(&mut self) {
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
    /// Where a loot roll at `origin` happens, for a mod's own loot kinds:
    /// the block entity it may copy from, and this level as
    /// `LootContext.getLevel` gives it (`Level.isRaining`: rain above 0.2;
    /// `isThundering`: thunder above 0.9; neither where `canHaveWeather` is
    /// false: a level with a ceiling or no sky light, or the End).
    pub fn loot_place_at(
        &self,
        origin: [f64; 3],
        block_entity: Option<sunrise_core::nbt::Tag>,
    ) -> sunrise_world::loot::LootPlace {
        let (block_entity, block_components) = match block_entity {
            Some(saved) => {
                let source = spill_common::source_snapshot(saved, None, None);
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
    /// The item entities `tick_dropped_items` took out, back in the level,
    /// before those its hooks spawned meanwhile (`addFreshEntity` appends).
    /// They keep their join numbers; the spawned ones got theirs on joining.
    pub fn put_back_dropped(&mut self, items: sunrise_sim::join_seq::JoinList<ItemEntity>) {
        self.dropped
            .restore(items, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
    }
    /// `AbstractCandleBlock.getParticleOffsets`, in sixteenths of a block:
    /// `CandleBlock.PARTICLE_OFFSETS` by its `candles` (one to four wicks),
    /// then `CandleCakeBlock.PARTICLE_OFFSETS`, whose one wick sits a block up.
    pub fn candle_offsets(name: &str, candles: Option<usize>) -> &'static [[u8; 3]] {
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
    /// A column whose surface is a full solid, non-fluid, non-leaf block open to the sky.
    pub fn land_column(&self, x: i32, z: i32) -> Option<i32> {
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
    /// `Blocks.AIR.defaultBlockState`: what a landing's destroy leaves.
    fn air_state(&self) -> BlockStateId {
        self.pack
            .assets
            .registry
            .default_state("minecraft:air")
            .unwrap_or(BlockStateId(0))
    }

    /// The cauldron at `pos`, and whether water is above it
    /// (`CauldronInteractions.isUnderWater`).
    pub fn cauldron_at(&self, pos: [i32; 3]) -> Option<(sunrise_world::chunk::BlockState, bool)> {
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

    pub fn bed_facing(&self, head: [i32; 3]) -> i32 {
        let state = self.world.block(head[0], head[1], head[2]);
        let facing = self
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("facing"))
            .map(String::as_str);
        sunrise_sim::mobs::villager::bed_facing_code(facing)
    }

    /// Clear all entity collections shared by server and client games.
    pub fn clear_shared_entities(&mut self) {
        self.mobs.clear();
        self.dropped.clear();
        self.orbs.clear();
        self.creatures.projectiles.clear();
        self.falling.clear();
        self.tnts.clear();
        self.boats.clear();
        self.carts.clear();
        self.armor_stands.clear();
        self.cushions.clear();
        self.item_frames.clear();
        self.paintings.clear();
        self.dragon.discard();
        self.crystals.clear();
        self.breath_clouds.clear();
        self.tridents.clear();
        self.rockets.clear();
        self.bolts.clear();
        self.ominous_spawners.clear();
        self.fangs.list.clear();
        self.potions.clear_thrown();
        self.end.clear_remote_eyes();
        self.riding = None;
        self.persist = Default::default();
        self.entity_attachments.clear();
    }

    /// The non-air blocks of a box in this world, with counts per block.
    pub fn blocks_json(&self, lo: [i32; 3], hi: [i32; 3]) -> serde_json::Value {
        let mut list = Vec::new();
        let mut census: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
        for y in lo[1]..=hi[1] {
            for z in lo[2]..=hi[2] {
                for x in lo[0]..=hi[0] {
                    let state = self.world.block(x, y, z);
                    if self.res.facts().is_air(state) {
                        continue;
                    }
                    let name = self.res.facts().get(state).name.to_string();
                    *census.entry(name).or_insert(0) += 1;
                    list.push(serde_json::json!([x, y, z, self.state_string(state)]));
                }
            }
        }
        serde_json::json!({ "from": lo, "to": hi, "counts": census, "blocks": list })
    }

    pub fn state_string(&self, state: BlockStateId) -> String {
        self.block_state_string(state).unwrap_or_else(|| "?".into())
    }

    pub fn vanilla_root(&self) -> PathBuf {
        if self.ui.vanilla_root.as_os_str().is_empty() {
            self.options.resources.clone()
        } else {
            self.ui.vanilla_root.clone()
        }
    }
}

pub trait GameGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// The level's dimension type: a mod's or a data pack's own when it has one, else the
    /// built-in level's.
    fn dimension_type<'__loan>(
        &'__loan self,
    ) -> &'__loan sunrise_world::dimension_type::DimensionFacts
    where
        '__sg: '__loan,
    {
        // A client of a server has the facts its server sent for the level, which are its own,
        // not the table the server's levels (in this process, for an integrated server) read.
        let remote = side_ops::world::SideOps::side_remote_dimension_context(self.side_state());
        if let Some((_, Some(facts))) = remote {
            return facts;
        }
        // Else the level the server named, whether or not the client has the mod that
        // registered it (`self.dim` is then the Overworld).
        let level = remote
            .map(|(name, _)| name)
            .filter(|level| !level.is_empty())
            .unwrap_or_else(|| self.dim.id());
        self.dimension_facts_for(
            sunrise_core::LevelKey::parse(level).unwrap_or(sunrise_core::LevelKey::OVERWORLD),
        )
    }
    /// A client only knows its current level's type; other keys use built-in defaults.
    fn dimension_facts_for<'__loan>(
        &'__loan self,
        key: sunrise_core::LevelKey,
    ) -> &'__loan sunrise_world::dimension_type::DimensionFacts
    where
        '__sg: '__loan,
    {
        use sunrise_world::dimension_type::DimensionFacts;
        if let Some((name, facts)) =
            side_ops::world::SideOps::side_remote_dimension_context(self.side_state())
        {
            if sunrise_core::LevelKey::parse(name).ok() == Some(key)
                && let Some(facts) = facts
            {
                return facts;
            }
            DimensionFacts::of_builtin_level(key)
        } else {
            DimensionFacts::of_level(key)
        }
    }
    fn outline_context_kind(&self) -> sunrise_world::collision_context::EntityContextKind {
        self.side_outline_camera_kind()
    }
    /// A block's outline boxes in world space (`getShape` plus the random offset).
    fn outline_boxes(&self, pos: [i32; 3]) -> Vec<[f64; 6]> {
        self.outline_boxes_in_context(&self.world, self.side_state().shulkers(), pos, true)
    }
    fn outline_boxes_in_context(
        &self,
        world: &World,
        shulkers: &sunrise_level::fast_hash::FxHashMap<
            [i32; 3],
            sunrise_level::shulker_box::Shulker,
        >,
        pos: [i32; 3],
        player_context: bool,
    ) -> Vec<[f64; 6]> {
        self.outline_boxes_iter(
            world,
            world.block(pos[0], pos[1], pos[2]),
            shulkers,
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
            self.selected_outline(&self.world, state, self.side_state().shulkers(), pos, true);
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
    fn sync_player_body(&mut self) {
        self.sync_main_hand();
        let glider = self.glider_worn();
        let sleeping = self.sleeping.is_some();
        let item_in_use = self
            .using
            .and_then(|(slot, _)| {
                if slot == held_items::OFFHAND_SLOT {
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
        let server_owns_glide =
            !side_ops::world::SideOps::side_client_has_integrated(self.side_state());
        let (common, state) = self.common_and_state_mut();
        let Some(player) = common.player.as_mut() else {
            return;
        };
        player.nudge = nudge;
        player.mobility_restricted = blind;
        player.yaw = common.yaw;
        player.pitch = common.pitch;
        player.auto_jump_enabled = state.auto_jump();
        player.item_use_speed = item_use_speed;
        player.item_use_slows_sprint = item_use_slows_sprint;
        player.respiration = respiration;
        player.glider = glider;
        // On someone else's server the glide flag is the server's
        // (`DATA_SHARED_FLAGS_ID`, applied by the SetEntityData handler); the
        // game's own integrated server has no such round trip, so the client
        // keeps predicting the clear a tick after the landing glide.
        player.server_owns_glide = server_owns_glide;
        let _ = player.set_sleeping(sleeping);
        player.leather_boots = leather_boots;
        player.freeze_immune = freeze_immune;
        player.fluids.fast_lava = fast_lava;
    }
    /// `Player.drop(stack, false, thrown_from_hand)`: the stack thrown from the eyes along the
    /// view, the main hand swinging for every drop (`LivingEntity.drop`; `PREDICTED` only keeps
    /// the swing from being sent back to the player who made it). `thrown_from_hand` names the
    /// player as the thrower, as the drop key does. The item's id, none for an empty stack or a
    /// toss a mod canceled.
    fn drop_stack_from_eyes(&mut self, stack: ItemStack, thrown_from_hand: bool) -> Option<u32> {
        self.side_drop_stack_from_eyes(stack, thrown_from_hand)
    }
    /// Run vanilla block behaviour against the world through `LevelAccess`.
    fn with_level<R>(
        &mut self,
        f: impl FnOnce(&Behaviours, &mut GameLevel<'_>, &mut Ticks) -> R,
    ) -> R {
        self.ensure_stateful_entity_table();
        let result = self.side_with_level_access(false, f);
        self.side_after_with_level();
        result
    }

    /// Placement queries need neither trigger snapshots nor live factory setup or effect drains.
    fn with_placement_preview_level<R>(
        &mut self,
        f: impl FnOnce(&Behaviours, &GameLevel<'_>, &Ticks) -> R,
    ) -> R {
        self.side_with_level_access(true, |behaviours, level, ticks| f(behaviours, level, ticks))
    }

    fn native_block_ignite_allowed(&mut self, pos: BlockPos, cause: &'static str) -> bool {
        self.with_level(|_, level, _| level.allow_block_ignite(pos, cause))
    }
    fn native_cauldron_level_change_allowed(
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
        self.side_process_level_events()
    }

    /// `Level.setBlock` with the full server available at the block-entity drop site.
    fn set_block_live(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        flags: i32,
        create_entity: bool,
    ) -> bool {
        self.side_set_block_live(pos, state, flags, create_entity)
    }
    fn attack_block(&mut self, pos: [i32; 3]) -> Result<()> {
        let note = self.with_level(|b, level, ticks| {
            b.attack_block(level, ticks, BlockPos::new(pos[0], pos[1], pos[2]))
        });
        // NoteBlock.attack awards the statistic.
        if note {
            self.side_award_custom_stat("play_noteblock", 1);
        }
        self.side_process_level_events()
    }
    /// `DistanceManager`'s tickets of the level in place, from every player in
    /// it (the local player, LAN guests, a server's players; a spectator the
    /// rule skips holds none), its forced chunks and its portal tickets. A game
    /// with neither a server nor a player (the headless fixtures) ticks
    /// everywhere.
    fn update_chunk_tickets(&mut self) {
        self.side_update_chunk_tickets();
    }

    /// Copies the moving-piston block entities into the world, where the
    /// collision queries and the renderer read them.
    ///
    /// A client runs this for every block change that ends a piston's move,
    /// so it stays linear in the pistons moving: a clock of a thousand pistons
    /// ends two thousand moves at once.
    fn sync_moving_pistons(&mut self) {
        use sunrise_level::fast_hash::{FxHashMap, FxHashSet};
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
                .or_insert_with(|| self.side_section_mesh_pending_in(section))
            {
                drawn.insert(pos);
                fallback.push((pos, piston));
            }
        }
        self.world.moving_piston_fallback = fallback;
        self.world.moving_pistons = current;
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
            sunrise_command::args::normalize(&mob.kind.id),
        );
        let Some(stack) = self.held().filter(|stack| stack.count > 0).cloned() else {
            return false;
        };
        let mut result = None;
        self.native_item_interact_mob(self.hand, entity, &kind, &stack, &mut result);
        result.is_some_and(|answer| self.interaction_consumed(answer))
    }
    /// Landing rules (farmland trampling, turtle eggs) wrote or removed the block
    /// raw: finish `Level.setBlock`'s neighbour updates, so a crop on trampled
    /// farmland breaks. The local player's and every server player's landings.
    /// The level events they send go out too: a turtle egg's break particles
    /// and sound (`decreaseEggs`' 2001, `Level.destroyBlock`'s for the last).
    ///
    /// A mod hears the change first (`native_entity_change_block_pre`, Bukkit's
    /// `EntityChangeBlockEvent`): cancelling puts the block back and skips the
    /// updates and the events, so nothing the change would have caused happens -
    /// the crop on trampled farmland stays, because breaking it is the
    /// neighbour update below and not a write of its own.
    ///
    /// The game event the rule posts comes back for the caller to post with the
    /// landing player in place (`post_landing_vibration`).
    fn landing_block_updates(
        &mut self,
        player: Option<u128>,
        event: &sunrise_sim::landing::FallEvent,
    ) -> Option<LandingVibration> {
        use sunrise_sim::landing::FallEvent;
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
                self.side_level_event_at(event, [position.x, position.y, position.z], data);
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
            self.side_level_event_at(2001, [position.x, position.y, position.z], from.0 as i32);
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
            .then_some(sunrise_sim::mobs::Who::Player);
        self.side_post_block_game_event(vibration.event, at, who, Some(vibration.state));
    }
    /// Whether a mod cancelled a landing's block change
    /// (`native_entity_change_block_pre`): asked with the two blocks' names and
    /// the player whose landing it was (`None` when no player did it).
    fn landing_change_cancelled(
        &mut self,
        player: Option<u128>,
        position: BlockPos,
        from: BlockStateId,
        to: BlockStateId,
    ) -> bool {
        self.side_landing_change_cancelled(player, position, from, to)
    }
    fn tick(&mut self) -> Result<()> {
        self.side_tick()
    }

    /// The brush's `ProjectileUtil.getHitResultOnViewVector` excludes only the source entity.
    fn mob_hit_within(&self, reach: f64) -> Option<(usize, f64, [f64; 3])> {
        let source = self.side_pick_camera_id();
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
    fn sweep_decision(&mut self, scale: f32, crit: bool, target: &str) -> bool {
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
                    && f.render_shape != sunrise_items::blocks::RenderShape::Model
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

    /// What `BlockTransformer.transformBlock` would do with a held axe, hoe
    /// or shovel on the block at `pos`, without doing it: the new state, its
    /// sound, loot table, the durability it costs, its particle and its
    /// transform type.
    fn block_transform(
        &self,
        stack: &ItemStack,
        hit: &Hit,
        sneaking: bool,
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
            && !sneaking;
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
        use sunrise_sim::block_transform::Around;
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
        let fallback;
        let transformers = match <Self::State as Side>::world_tables(self.side_state()) {
            Some(tables) => &*tables.transformers,
            None => {
                fallback = self.res.transformers();
                &*fallback
            }
        };
        let vanilla = transformers
            .apply(self.items(), registry, &id, face_name, &around)
            .and_then(|(_, t, state)| Some((registry.resolve(&state).ok()?, t)));
        let result = |new, t: &sunrise_sim::block_transform::Transform| {
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
                return Some(waxing_common::tool_ability_result(new, ability));
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
                return Some(waxing_common::tool_ability_result(new, ability));
            }
        }
        None
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
    fn can_place_used_fire(&mut self, target: [i32; 3]) -> bool {
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
    /// `Player.giveExperiencePoints` changes the score before the XP bar and reports level changes once.
    fn give_experience_points(&mut self, points: i32) {
        self.side_give_experience_points(points);
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameGameExt1<'__sg> for G {}

/// `mark_motion` ids of entities that aren't mobs or dropped items (boats, minecarts, armor
/// stands, item frames, paintings, primed TNT, falling blocks, rockets, tridents, thrown
/// potions): their entity id under this tag.
pub const MOTION_OBJECT: u64 = 3 << 32;

/// A block entity, by its position (`motion_block_entity`).
pub const MOTION_BLOCK_ENTITY: u64 = 5 << 32;

/// The player's own model (third person, and a shader pack's shadow pass).
pub const MOTION_OWN_PLAYER: u64 = 7 << 32;

/// winit's events for a window's game, with any loop: a headless client (#4951) has no
/// winit loop and calls these from its own.
impl crate::game::CommonGame<'_> {}

// Out-of-line init bundles for wrapped Game fields so finish_construct's unoptimised stack stays small.

pub struct WrappedPrepInit {
    pub item_features: sunrise_items::creative_catalog::ItemFeatureRequirements,
    pub level: Level,
    pub painting_variants: sunrise_gameplay_common::painting_variants::PaintingVariants,
    pub world: World,
    pub now: Instant,
    pub pack: PackResources,
    pub options: Options,
    pub clock: FixedStep,
    pub tick: sunrise_sim::parallel::TickThread,
    pub ui: chat_common::WorldCommandState,
}

// Const-generic impl keeps constructor helpers out of line without creating replacement slots.
pub struct ConstructInit<const NO_SLOT: bool>;

impl<const NO_SLOT: bool> ConstructInit<NO_SLOT> {
    #[inline(never)]
    fn build_loop_context(
        res: &Resources,
        loop_side: sunrise_world::native_load_context::LoopSide,
    ) -> Arc<sunrise_world::native_load_context::LoopContext> {
        Arc::new(res.core.item_data.context().loop_context(loop_side, []))
    }

    #[inline(never)]
    #[allow(clippy::boxed_local)]
    pub fn assemble_common<'a>(
        res: &'a Arc<Resources>,
        wp: Box<WrappedPrepInit>,
    ) -> CommonGame<'a> {
        let loop_side = wp.options.execution_role.loop_side();
        let mod_sources = wp.options.mods.clone();
        let creative = wp.options.creative;
        let inventory = vec![None; 36];
        let look = wp.options.look;
        let yaw = look.map_or(wp.level.rotation[0], |l| l.0);
        let pitch = look.map_or(wp.level.rotation[1], |l| l.1);
        CommonGame {
            world_biomes: None,
            world_biomes_base: None,
            res: res.clone(),
            rest_loader: None,
            collision: res.collision(),
            motion: res.motion(),
            health_resources: res.motion().health.clone(),
            damage_tag_generation: 1,
            pack: wp.pack,
            world: wp.world,
            options: wp.options,
            player: None,
            spawn: None,
            clock: wp.clock,
            previous_position: Vec3::ZERO,
            eye: 1.62,
            yaw,
            pitch,
            inventory,
            selected: 0,
            carried: None,
            craft_grid: vec![None; 9],
            craft_size: 2,
            container: None,
            station: None,
            potions: Default::default(),
            scratch_slot: None,
            mining: None,
            use_delay: 0,
            creative,
            mode: if creative {
                sunrise_command::args::GameMode::Creative
            } else {
                sunrise_command::args::GameMode::Survival
            },
            pause: false,
            previous: wp.now,
            ticks: 0,
            error: None,
            level: wp.level,
            loop_context: Self::build_loop_context(res, loop_side),
            tick: wp.tick,
            native_settings: mod_sources.settings.clone(),
            damage_hooks: native_hooks::DAMAGE_HOOKS,
            dropped: Default::default(),
            mobs: Default::default(),
            mob_index: Default::default(),
            creatures: Default::default(),
            using: None,
            use_duration: 0,
            use_start: Default::default(),
            armor: [None, None, None, None],
            offhand: None,
            equipment: Default::default(),
            hand: Hand::Main,
            cooldowns: Default::default(),
            autosaving: false,
            experience: Experience::default(),
            player_score: 0,
            orbs: Default::default(),
            swing: Default::default(),
            swings_started: 0,
            clicks: Default::default(),
            flash: 0,
            sleeping: None,
            sleep_timer: 0,
            dim: Dim::Overworld,
            world_flat: false,
            portal_player: Default::default(),
            portal_sections: Default::default(),
            end: Default::default(),
            block_ticks: Ticks::new(0),
            level_events: Vec::new(),
            falling: Default::default(),
            paintings: Default::default(),
            painting_variants: wp.painting_variants,
            item_frames: Default::default(),
            armor_stands: Default::default(),
            cushions: Default::default(),
            boats: Default::default(),
            carts: Default::default(),
            bolts: Default::default(),
            maps: maps::Maps::default(),
            dragon: Default::default(),
            crystals: Default::default(),
            breath_clouds: Default::default(),
            riding: None,
            posed_rider: None,
            pending: Default::default(),
            hook: Default::default(),
            tridents: Default::default(),
            rockets: Default::default(),
            ominous_spawners: Default::default(),
            tnts: Default::default(),
            transactions: Default::default(),
            spawner_tick_positions: Vec::new(),
            trial_spawner_tick_positions: Vec::new(),
            loop_states: Default::default(),
            item_features: wp.item_features,
            creative_contents: Default::default(),
            entity_initialization: Default::default(),
            kinetic_feedback: HashMap::new(),
            control: None,
            events: Default::default(),
            mods: mod_sources,
            recipe_tables: Default::default(),
            native_lifecycle: Default::default(),
            resident_entities: Default::default(),
            config_root: mod_api::__private::RegistrationData::resolved_config_dir(),
            native_configs: Default::default(),
            native_net: Default::default(),
            sky_darken_memo: Default::default(),
            persist: Default::default(),
            entity_attachments: Default::default(),
            collider_placements: Default::default(),
            attachment_indexes: Default::default(),
            passages: Default::default(),
            previous_mode: None,
            entity_reach: 3.0,
            block_reach: 4.5,
            player_state: Default::default(),
            server_items: None,
            ui: wp.ui,
            mount: Default::default(),
            stats: Default::default(),
            score: Default::default(),
            border: Default::default(),
            adv: Default::default(),
            fangs: Default::default(),
        }
    }
}

pub use launch_api::{UsageError, exit_code};

/// Every launch reads the prepared game files (`sunrise-prepare`'s output, whose
/// manifest is `sunrise-resources.json`): without them, say where they were
/// looked for and how to get them, before a window or a loader starts. `get` is
/// the page a hook sends the player to (`sunrise_hooks::engine::get_the_game`).
pub fn preflight_resources(
    root: &std::path::Path,
    get: Option<sunrise_hooks::GetTheGame>,
    dedicated: bool,
) -> std::result::Result<sunrise_core::prepared_profile::Profile, String> {
    use sunrise_core::{
        prepared_profile::{MANIFEST, Manifest, Profile},
        version::VERSION_ID,
    };

    if !root.join(MANIFEST).is_file() {
        let launcher = get.filter(|_| !dedicated).map_or(String::new(), |get| {
            format!(
                "Sunrise is started from its launcher: download it at {} (Download). It needs \
                 your own Minecraft: Java Edition; the launcher signs you in and prepares the \
                 game files.\nDevelopers: ",
                get.address
            )
        });
        let shown = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
        let profile = if dedicated {
            Profile::Server
        } else {
            Profile::Full
        };
        return Err(format!(
            "Sunrise's prepared game files are missing: no {MANIFEST} in {}\n\
             {launcher}{}, or pass --resources <folder>",
            shown.display(),
            profile.repair(root),
        ));
    }
    let manifest = Manifest::read(root).map_err(|problem| {
        format!(
            "Sunrise's prepared game files at {} are damaged: {problem}",
            root.display(),
        )
    })?;
    let profile = manifest.profile();
    if profile == Profile::Server && !dedicated {
        return Err(format!(
            "{} contains server-only game files, not a full client preparation. Run \
             `sunrise-prepare --prepare --server-jar PATH --output FULL_DIR` with your own \
             client files, then pass --resources FULL_DIR.",
            root.display(),
        ));
    }
    let mut problems = manifest.version_problems(root, VERSION_ID);
    if profile == Profile::Server {
        problems.extend(manifest.server_inventory_problems(root));
    }
    // An empty JSON file left by an interrupted export fails with a generic
    // parse error later. Name the file now, before starting a loader or a window.
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension() == Some(std::ffi::OsStr::new("json"))
                && std::fs::metadata(&path).is_ok_and(|meta| meta.is_file() && meta.len() == 0)
            {
                problems.push(format!("{} is empty", entry.file_name().to_string_lossy()));
            }
        }
    }
    if problems.is_empty() {
        Ok(profile)
    } else {
        Err(format!(
            "Sunrise's prepared game files in {} are stale or damaged:\n  {}\n\
             Prepare them again with `{}`.",
            root.display(),
            problems.join("\n  "),
            profile.repair(root),
        ))
    }
}

/// The most `Player.sleepCounter` reaches: 100 asleep, then up to here over the
/// waking fade.
pub const MAX_SLEEP_TIMER: i32 = 109;

// Player.tick: cap the sleeping fade at 100, then fade out over 10 waking ticks.
pub fn tick_sleep_timer(timer: i32, sleeping: bool) -> i32 {
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
pub fn outline_stop(
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
pub fn clip_start_source(
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

pub use closest_space::closest_space_nudge;

pub use sunrise_gameplay_common::game_master_blocks::is_game_master_block;
#[cfg(any(test, feature = "test-support"))]
pub mod test_neighbourhood {
    //! Shared fixture step for tests that tick entities on prepared data.

    impl crate::game::CommonGame<'_> {
        /// Loads the chunks within 2 columns of `(cx, cz)` that are not loaded yet.
        /// Entity ticking waits for every chunk in that 5x5 neighbourhood
        /// (`ChunkMap.prepareEntityTickingChunk`), so a fixture that ticks mobs,
        /// items or TNT in a chunk calls this after setting up its own chunks.
        pub fn load_ticking_neighbourhood(&mut self, cx: i32, cz: i32) {
            for z in cz - 2..=cz + 2 {
                for x in cx - 2..=cx + 2 {
                    if self.world.chunks.contains_key(&(x, z)) {
                        continue;
                    }
                    self.world.chunks.insert(
                        (x, z),
                        std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(x, z)),
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
#[cfg(any(test, feature = "test-support"))]
pub fn test_server_load() -> sunrise_world::native_load_context::LoadContext {
    use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs};
    LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    )
}

#[cfg(test)]
pub mod nested_orb_spawn_tests {}

#[cfg(any(test, feature = "test-support"))]
pub fn in_own_process(name: &str, body: impl FnOnce()) {
    const OWN_PROCESS: &str = "SUNRISE_TEST_OWN_PROCESS";
    let name = test_fixtures::child_test_name(name);
    if std::env::var_os(OWN_PROCESS).is_some_and(|v| v == name.as_ref()) {
        return body();
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            name.as_ref(),
            "--exact",
            "--include-ignored",
            "--test-threads=1",
        ])
        .env(OWN_PROCESS, name.as_ref());
    let out = test_fixtures::output_within(&mut command, &name, "isolated");
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
#[allow(unused_imports)]
pub use {
    copper_golems::golem_preserved_equipment,
    native_datafix::upgrade_nbt,
    native_hooks::{
        authoritative_world_for_mods, mod_data_root, startup_math_profile, world_for_mods,
    },
    option_paths::{game_dir, options_path},
    remote::{inventory_screen_slot, inventory_screen_slot_with_type, remote_station_kind},
    stacked_on_other::{
        stacked_listened, stacked_listened_for_side, stacked_listens_for, stacked_listens_for_side,
    },
};

#[path = "game/world/container_loot_level.rs"]
pub mod container_loot_level;

#[path = "game/world/end_state.rs"]
pub mod end_state;

#[path = "game/world/entity_memory.rs"]
pub mod entity_memory;

#[path = "game/world/world_preload.rs"]
pub mod world_preload;

#[path = "game/world/packs_common.rs"]
pub mod packs_common;

#[path = "game/world/integrated_link.rs"]
pub mod integrated_link;

#[path = "game/world/owner_link.rs"]
pub mod owner_link;

#[path = "game/runtime/events_common.rs"]
pub mod events_common;

#[path = "game/runtime/resource_cpu_common.rs"]
pub mod resource_cpu_common;

#[path = "game/world/world_rest_common.rs"]
pub mod world_rest_common;

#[path = "game/blocks/maps/data.rs"]
pub mod maps;

#[path = "game/entities/entity_types.rs"]
pub mod entity;

#[path = "game/entities/general.rs"]
pub mod general;

#[path = "game/modding/pack_finders/mod.rs"]
pub mod pack_discovery;

#[path = "game/interface/scoreboard/common.rs"]
pub mod scoreboard_common;

#[path = "game/interface/advancements/common.rs"]
pub mod advancements_common;

#[path = "game/administration/function_pack_stack.rs"]
pub mod function_pack_stack;

#[path = "game/network/lan_common.rs"]
pub mod lan_common;

#[path = "game/network/lan_menu_data.rs"]
pub mod lan_menu_data;

#[path = "game/network/lan_mob_metadata.rs"]
pub mod lan_mob_metadata;

#[path = "game/network/lan_particle_data.rs"]
pub mod lan_particle_data;

#[path = "game/network/lan_station_data.rs"]
pub mod lan_station_data;

#[path = "game/network/untrusted_uri.rs"]
pub mod untrusted_uri;

#[path = "game/network/villager_metadata.rs"]
pub mod villager_metadata;

#[path = "game/blocks/machine_containers.rs"]
pub mod machine_containers;

#[path = "game/blocks/piston_carry_common.rs"]
pub mod piston_carry_common;

#[path = "game/blocks/spill_common.rs"]
pub mod spill_common;

#[path = "game/blocks/sculk_control_common.rs"]
pub mod sculk_control_common;

#[path = "game/blocks/waxing_common.rs"]
pub mod waxing_common;

#[path = "game/blocks/cauldron_interactions.rs"]
pub mod cauldron_interactions;

#[path = "game/blocks/composter_data.rs"]
pub mod composter_data;

#[path = "game/blocks/world_fluid_data.rs"]
pub mod world_fluid_data;

#[path = "game/blocks/growth_features.rs"]
pub mod growth_features;

#[path = "game/items/item_use_common.rs"]
pub mod item_use_common;

#[path = "game/items/crossbow_common.rs"]
pub mod crossbow_common;

#[path = "game/items/deflect_common.rs"]
pub mod deflect_common;

#[path = "game/items/mace_common.rs"]
pub mod mace_common;

#[path = "game/items/fishing_common.rs"]
pub mod fishing_common;

#[path = "game/items/potions_common.rs"]
pub mod potions_common;

#[path = "game/items/totem_common.rs"]
pub mod totem_common;

#[path = "game/items/combat_common.rs"]
pub mod combat_common;

#[path = "game/items/interactions_common.rs"]
pub mod interactions_common;

#[path = "game/items/trading_common.rs"]
pub mod trading_common;

#[path = "game/modding/journal_level_effects.rs"]
pub mod journal_level_effects;

#[path = "game/modding/native_menu_values.rs"]
pub mod native_menu_values;

#[path = "game/modding/native_menu_ids.rs"]
pub mod native_menu_ids;

#[path = "game/modding/native_menu_data.rs"]
pub mod native_menu_data;

#[path = "game/modding/player_state.rs"]
pub mod player_state;

#[path = "game/modding/native_data_maps_common.rs"]
pub mod native_data_maps_common;

#[path = "game/modding/item_particle_send.rs"]
pub mod item_particle_send;

#[path = "game/modding/native_world_read.rs"]
pub mod native_world_read_common;

#[path = "game/modding/native_block_entity_read.rs"]
pub mod native_block_entity_read;

#[path = "game/modding/native_rain_read.rs"]
pub mod native_rain_read;

#[path = "game/modding/native_equipment_read.rs"]
pub mod native_equipment_read;

#[path = "game/modding/native_structure_values.rs"]
pub mod native_structure_values;

#[path = "game/modding/native_entity_motion_values.rs"]
pub mod native_entity_motion_values;

#[path = "game/modding/native_collider_state.rs"]
pub mod native_collider_state;

#[path = "game/modding/native_recipe_tables.rs"]
pub mod native_recipe_tables;

#[path = "game/modding/native_net_common.rs"]
pub mod native_net_common;

#[path = "game/modding/dragon_parts_table.rs"]
pub mod dragon_parts_table;

#[path = "game/modding/attachment_indexes.rs"]
pub mod attachment_indexes;

#[path = "game/modding/native_level_refs_common.rs"]
pub mod native_level_refs_common;

#[path = "game/modding/native_entity_base.rs"]
pub mod native_entity_base;

#[path = "game/modding/native_entity_state_common.rs"]
pub mod native_entity_state_common;

#[path = "game/modding/native_config_file_services.rs"]
pub mod native_config_file_services;

#[path = "game/modding/entity_part_state_common.rs"]
pub mod entity_part_state_common;

#[path = "game/modding/native_passages.rs"]
pub mod native_passages;

#[path = "game/entities/collider_rays.rs"]
pub mod collider_rays;

#[path = "game/entities/armor_stands_common.rs"]
pub mod armor_stands_common;

#[path = "game/entities/cushions_common.rs"]
pub mod cushions_common;

#[path = "game/entities/entity_sounds_common.rs"]
pub mod entity_sounds_common;

#[path = "game/entities/frames_common.rs"]
pub mod frames_common;

#[path = "game/entities/lightning_common.rs"]
pub mod lightning_common;

#[path = "game/entities/seats_common.rs"]
pub mod seats_common;

#[path = "game/entities/tnt_common.rs"]
pub mod tnt_common;

#[path = "game/mobs/creaking_common.rs"]
pub mod creaking_common;

#[path = "game/mobs/wither_common.rs"]
pub mod wither_common;

#[path = "game/interface/profile_capture_common.rs"]
pub mod profile_capture_common;

#[path = "game/interface/level_sounds_common.rs"]
pub mod level_sounds_common;

#[path = "game/interface/ray_shapes.rs"]
pub mod ray_shapes;

#[path = "game/interface/streaming_radius.rs"]
pub mod streaming_radius;

#[path = "game/interface/weather_height.rs"]
pub mod weather_height;

#[path = "game/interface/name_component.rs"]
pub mod name_component;

#[path = "game/interface/option_paths.rs"]
pub mod option_paths;

use self::item_abilities::ItemAbilitiesGameExt1 as _;
use self::transactions::entities::EntityRemovalGameExt as _;

pub trait GameGameExt3<'__sg>: crate::game::SideGame<'__sg> {
    fn resident_source_target_visible(
        &self,
        dimension: mod_api::DimensionKey,
        target: commands_common::Target,
    ) -> bool {
        let player = match target {
            commands_common::Target::Guest(conn) => Some(conn),
            commands_common::Target::Player => crate::game::Side::lan_acting(self.side_state()),
            _ => None,
        };
        if player
            .and_then(|conn| self.command_guest(conn))
            .is_some_and(|guest| !guest.body_present())
        {
            return false;
        }
        if dimension == self.dim.key() {
            return !self.target_removed(&target);
        }
        let dim = match dimension {
            mod_api::DimensionKey::OVERWORLD => Dim::Overworld,
            mod_api::DimensionKey::NETHER => Dim::Nether,
            mod_api::DimensionKey::END => Dim::End,
            key => Dim::Extra(key),
        };
        side_ops::world::SideOps::side_parked_target_visible(self.side_state(), dim, &target)
            .is_none_or(|visible| visible && !self.removal_pending_in(dim, &target))
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameGameExt3<'__sg> for G {}

pub use side_ops::blocks::BlocksSideGameExt as _;
pub use side_ops::events::EventsSideGameExt as _;
pub use side_ops::items::ItemsSideGameExt as _;
pub use side_ops::lan::LanSideGameExt as _;
pub use side_ops::mobs_creatures::MobsCreaturesSideGameExt as _;
pub use side_ops::mobs_rest::MobsRestSideGameExt as _;
pub use side_ops::modapi::ModapiSideGameExt as _;
pub use side_ops::modapi_hooks::ModapiHooksSideGameExt as _;
pub use side_ops::modapi_modding::ModapiModdingSideGameExt as _;
pub use side_ops::player::PlayerSideGameExt as _;
pub use side_ops::presentation::PresentationSideGameExt as _;
pub use side_ops::tables::TablesSideGameExt as _;
pub use side_ops::ui::UiSideGameExt as _;
pub use side_ops::world::WorldSideGameExt as _;
pub use tables::GameItems as _;
pub use tables::WorldTablesExt as _;
pub use transactions::TransactionsGameExt as _;
pub use weather::WeatherGameExt as _;

pub trait SideGame<'g>:
    std::ops::DerefMut<Target = CommonGame<'g>>
    + sunrise_sim::crafting::CraftingRemainder
    + sunrise_sim::menu::SlotRules
    + Sized
{
    type State: Side<Nominal<'g> = Self>
        + crate::game::side_ops::blocks::SideOps<Game<'g> = Self>
        + crate::game::side_ops::events::SideOps<Game<'g> = Self>
        + crate::game::side_ops::items::SideOps<Game<'g> = Self>
        + crate::game::side_ops::lan::SideOps<Game<'g> = Self>
        + crate::game::side_ops::mobs_creatures::SideOps<Game<'g> = Self>
        + crate::game::side_ops::mobs_rest::SideOps<Game<'g> = Self>
        + crate::game::side_ops::modapi::SideOps<Game<'g> = Self>
        + crate::game::side_ops::modapi_hooks::SideOps<Game<'g> = Self>
        + crate::game::side_ops::modapi_modding::SideOps<Game<'g> = Self>
        + crate::game::side_ops::player::SideOps<Game<'g> = Self>
        + crate::game::side_ops::presentation::SideOps<Game<'g> = Self>
        + crate::game::side_ops::tables::SideOps<Game<'g> = Self>
        + crate::game::side_ops::ui::SideOps<Game<'g> = Self>
        + crate::game::side_ops::world::SideOps<Game<'g> = Self>;
    fn common(&self) -> &CommonGame<'g> {
        self
    }
    fn common_mut(&mut self) -> &mut CommonGame<'g> {
        self
    }
    fn side_state(&self) -> &Self::State;
    fn side_state_mut(&mut self) -> &mut Self::State;

    fn host_side(&mut self) -> mod_api::HostSide<'_> {
        Self::State::host_side(self)
    }
    fn as_host(&self) -> &dyn mod_api::Host {
        Self::State::as_host(self)
    }
    fn common_and_state_mut(&mut self) -> (&mut CommonGame<'g>, &mut Self::State);
}

use crate::game::clip_ray::ClipRayGameExt1 as _;

use crate::game::commands_common::CommandsCommonGameExt2 as _;

use crate::game::creatures::ModGameExt1 as _;

use crate::game::held_items::HeldItemsGameExt1 as _;

use crate::game::native_hooks::SharedGameExt1 as _;

use crate::game::native_hooks::item_interact_mob::GameHooks as _;

use crate::game::native_hooks::sweep_attack::GameHooks as _;

use crate::game::native_hooks::use_item_hand::UseItemHandGameExt1 as _;

use crate::game::native_state_shapes::NativeStateShapesGameExt1 as _;

use crate::game::pick_ray::PickRayGameExt1 as _;

use crate::game::player_identity::PlayerIdentityGameExt1 as _;

use crate::game::tool_modification::ToolModificationGameExt1 as _;

#[path = "game/server_side.rs"]
pub mod server_side;

#[path = "game/client_side.rs"]
pub mod client_side;

#[path = "game/modding/native_registry_entries.rs"]
pub mod native_registry_entries;

#[path = "game/modding/native_resources.rs"]
pub mod native_resources;

#[cfg(any(test, feature = "test-support"))]
pub mod test_fixtures;

#[cfg(test)]
mod native_entity_refs {
    #[cfg(test)]
    pub use super::test_fixtures as tests;
}

#[cfg(any(test, feature = "test-support"))]
#[path = "game/network/lan_meta_fixture.rs"]
pub mod lan_meta_fixture;
