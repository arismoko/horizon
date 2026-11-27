//! Native hook implementations, split by catalog area.
//!
//! Hook documentation and contract versions live in the catalog, not a shared changelog.
//!
//! Native mods are built against an exact game revision, rather than a hook API revision.
//! Bodies have no `cfg` and no macros: horizon-mixins refuses a target with
//! either (`tests::hook_points_stay_hookable`).

/// Generated dispatch entry points, not hand-written hook implementations.
pub mod fire;
use self::bonemeal::GameHooks as _;
#[allow(unused_imports)]
pub use block_entity_read::ResidentReadBlockEntity;
#[allow(unused_imports)]
pub use build_creative_mode_tab_contents::CreativeCache;
#[allow(unused_imports)]
pub use fire::{ConnectsToCall, ItemAfterVerticalCollisionCall, UpdateShapeCall};
pub use mod_api::{BlockEntityDataSource, BlockEntityRef, ClockAdjustment};

pub use super::{
    GameLevel, LevelDimension, Screen,
    execution_role::{self, ExecutionRole},
    mod_api_host,
};
pub use crate::game::native_data_maps_common::DataMapType;
pub use mod_api::__private::game::{
    BlockStateDefinitionRegistration, ItemModelRegistration, ItemTintSourceRegistration,
    JsonAnimationTypeRegistration, LayerDefinitionRegistration, ModelLoaderRegistration,
    ParticleGroupRegistration, PresetEditorFactoryRegistration, RangeSelectPropertyRegistration,
    SelectPropertyRegistration, SpriteSourceRegistration, StandaloneModelRegistration,
};
pub use mod_api::GameMode;
pub use mod_api::GameTests;
#[allow(unused_imports)]
pub use mod_api::game::{PhantomSpawnResult, SleepProblem};
pub use mod_api::{
    self, Answered, BlockTintSources, CauldronFluidContents, CauldronInteractions,
    ClientMiningPrediction, ClientReloadListeners, ColorResolvers, EnchantmentEffects,
    EnchantmentInfo, EntityLayers, EntityRef, EntityRenderers, ExperienceOrbRef, GuiLayerRegistrar,
    HandDrawContext, HandRender, InsideEffects, InteractionAttempt, InteractionKey, ItemEntityRef,
    LifecycleContext, LivingRef, LoadConditions, NoteBlockChange, ParticleProviders,
    PathComputationType, Placement, PlayerRef, PoiTypeExtensions, RecipeBookCategory,
    ScreenOpening, ScreenWidgets, ScreenshotImage, ServerReloadListeners, ServerTickPhase,
    SpecialModelRenderers, ToolUse, VillagerProfessions,
};
pub use mod_api::{
    ActiveEffect, AttachmentStore, BlockClick, BlockUpdateFlags, BonemealSource, ChunkRef,
    FluidStack, ItemClick, LevelRef, LogicalSide, NeighbourUpdate, TooltipElement, TriState,
    UsePhase,
};
pub use mod_api::{
    BlockKey, DebugEntry, DebugRenderer, EffectKey, FrameModel, FrameModelEdit, MapDecorationType,
    NameTagLine, SkullModel, TextureAtlasRegistration,
};
#[allow(unused_imports)]
pub use mod_api::{
    BlockRim, ClientPacket, ClientPayload, ColoredLight, ContainerLootFilled, ExplosionStart,
    ItemFished, LootDrops, PanelEvent, PortalShape, ScreenView,
};
pub use mod_api::{Despawn, Identifier, MobRef, PermissionLevel, RailShape};
pub use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
pub use sunrise_core::{
    connection_kind::ConnectionKind,
    geometry::Direction,
    position::BlockPos,
    ticks::{ScheduleResult, TickPriority},
};
pub use sunrise_game_data::gamerules::Value as GameRuleValue;
pub use sunrise_game_data::suggestion::Suggestion;
#[cfg(test)]
pub use sunrise_sim::mobs::Damage;
pub use sunrise_world::level::BonemealCheck;
pub use {
    sunrise_core::{
        nbt::{Compound, Tag},
        use_animation::UseAnimation,
    },
    sunrise_world::{
        fluid::FluidKind,
        level::{LevelAccess, PlacementContext, Ticks},
        registry::{BlockRegistry, BlockStateId},
    },
};
pub use {
    sunrise_game_data::text::Text, sunrise_items::items::ItemStack,
    sunrise_level::biome::BiomeColors, sunrise_level::biome::SpawnEntry,
    sunrise_level::block_entities::Pos, sunrise_level::containers::BlockEntity,
    sunrise_level::containers::MOD_MENUS, sunrise_level::containers::ModBlockEntityFields,
    sunrise_level::fast_hash::FxHashMap, sunrise_level::particle_options::ParticleOptions,
    sunrise_level::particle_options::ParticleType, sunrise_mod_package::mod_list,
    sunrise_sim::crafting::Ingredient, sunrise_sim::crafting::recipe_result,
    sunrise_sim::first_person_sim::Hand, sunrise_sim::health::Attack,
    sunrise_sim::health::DamageContainer, sunrise_sim::health::DamageHooks,
    sunrise_sim::health::DamageLevel, sunrise_sim::health::HurtSource,
    sunrise_sim::health::Hurting, sunrise_sim::health::Knock, sunrise_sim::health::ShieldBlock,
    sunrise_sim::health::Victim, sunrise_sim::mobs::Who,
};

pub mod add_attribute_tooltips;
pub mod add_client_reload_listeners;
pub mod add_pack_finders;
pub mod add_section_geometry;
pub mod add_server_reload_listeners;
pub mod adjacent_block_path_type;
pub mod affect_neighbors_after_removal;
pub mod animal_tame;
pub mod animate_tick;

/// The hook points. Each does nothing in Sunrise.
pub trait ModGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Shared click metadata borrows the already-converted API snapshot for this call.
    fn block_click<'a>(
        &mut self,
        hit: &super::Hit,
        hand: mod_api::Hand,
        stack: Option<&'a mod_api::ItemStack>,
    ) -> mod_api::BlockClick<'a> {
        let (player, player_entity_id) = self.native_clicker();
        mod_api::BlockClick::from(mod_api::__private::built::BlockClick {
            side: if self.is_logical_server() {
                LogicalSide::Server
            } else {
                LogicalSide::Client
            },
            dimension: self.dim.level(),
            pos: hit.pos,
            hit: hit.native(),
            hand,
            stack,
            player,
            player_entity_id,
        })
    }
    /// The living entity an effect decision names: `Some(id)` a mob's
    /// `Entity.getId`, `None` the acting player. `None` when no native
    /// world runs, and the decision falls back to vanilla's.
    fn effect_living_ref(&self, entity: Option<i32>) -> Option<LivingRef> {
        let epoch = self.native_lifecycle.epoch()?;
        let id = match entity {
            Some(id) => id as u32,
            None => self.own_entity_id() as u32,
        };
        Some(LivingRef::new(EntityRef::new(epoch, id)))
    }
    /// `BonemealEvent` for bone meal from `stack` on `pos`, used by the
    /// acting player (`by_player`) or by a dispenser or a villager:
    /// `Some(successful)` when a mod canceled it, the answer `growCrop`
    /// gives in vanilla's place. The caller keeps what the mods did to
    /// `stack`.
    fn bonemeal(&mut self, pos: BlockPos, by_player: bool, stack: &mut ItemStack) -> BonemealAsk {
        if !mod_api::__private::listened(mod_api::HookId::Bonemeal) {
            return BonemealAsk {
                crop: None,
                check: None,
            };
        }
        let client = self.side_has_remote();
        let check = self.with_level(|b, level, _| b.is_valid_bonemeal_target(level, pos, client));
        let valid = check.is_valid();
        let state = self.world.block(pos.x, pos.y, pos.z);
        let (player, player_entity_id) = if by_player {
            let (uuid, entity) = self.native_clicker();
            (Some(uuid), Some(entity))
        } else {
            (None, None)
        };
        let (mut successful, mut canceled) = (false, false);
        let mut snapshot = Some(stack.clone());
        self.native_bonemeal(crate::game::native_hooks::fire::BonemealCall {
            dimension: self.dim.level(),
            player,
            player_entity_id,
            pos,
            state,
            source: BonemealSource::Interaction,
            stack: &mut snapshot,
            valid_bonemeal_target: valid,
            successful: &mut successful,
            canceled: &mut canceled,
        });
        match snapshot {
            Some(next) => *stack = next,
            None => stack.count = 0,
        }
        BonemealAsk {
            crop: canceled.then_some(successful),
            check: Some(check),
        }
    }
    /// The acting player as the hook points name one: the guest whose turn
    /// it is, else the host.
    fn native_acting_id(&mut self) -> u128 {
        self.side_acting_id()
    }
    /// The player clicking, as the block-use hooks name them: the acting
    /// player's UUID and entity id on a server, this client's own player on
    /// a client.
    fn native_clicker(&mut self) -> (u128, i32) {
        self.side_clicker()
    }
    /// `PlayerInteractEvent.RightClickBlock` for a click on `hit` with
    /// `hand` holding `stack`: what the mods let it use.
    fn right_click_block(
        &mut self,
        hit: &super::Hit,
        hand: Hand,
        stack: Option<&ItemStack>,
    ) -> BlockClickAnswer {
        let (mut use_block, mut use_item) = (TriState::Default, TriState::Default);
        if !mod_api::__private::listened(mod_api::HookId::RightClickBlock) {
            return BlockClickAnswer::Go {
                use_block,
                use_item,
            };
        }
        let Ok(snapshot) = stack.try_into_api() else {
            return BlockClickAnswer::Go {
                use_block,
                use_item,
            };
        };
        let click = self.block_click(hit, hand, snapshot.as_ref());
        let (mut result, mut canceled) = (mod_api::InteractionResult::Pass, false);
        self.native_right_click_block(
            &click,
            &mut use_block,
            &mut use_item,
            &mut result,
            &mut canceled,
        );
        if canceled {
            BlockClickAnswer::Canceled(result)
        } else {
            BlockClickAnswer::Go {
                use_block,
                use_item,
            }
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> ModGameExt1<'__sg> for G {}

pub mod anvil_update;
pub mod apply_enchantments;
pub mod arm_pose_transform;
pub mod armor_hurt;
pub mod arrow_nock;
pub mod baby_entity_spawn;
pub mod base;
pub mod beacon_color_multiplier;
pub mod block_drops;
pub mod block_entity_load_additional;
pub mod block_entity_neighbors;
pub mod block_entity_on_load;
pub mod block_entity_on_load_routes_tests;
pub mod block_entity_on_load_tests;
pub mod block_entity_read;
pub mod block_entity_save_additional;
pub mod block_entity_save_additional_tests;
pub mod block_entity_type_add_blocks;
pub mod block_entity_view_distance;
pub mod block_grow_feature;
pub mod block_landing;
pub mod block_on_place;
pub mod block_particle_effects;
#[cfg(any())]
#[cfg(any(test, feature = "test-support"))]
pub mod block_particle_effects_tests;
pub mod block_path_type;
pub mod block_render_visibility;
pub mod block_sticky;
pub mod block_tool_modification;
pub mod block_use_item_on;
pub mod block_use_without_item;
pub mod bonemeal;
pub mod bounce_restitution;
pub mod break_progress_positions;
pub mod build_creative_mode_tab_contents;
pub mod calculate_player_turn;
pub mod camera_math;
pub mod camera_read;
pub mod camera_submersion;
pub mod camera_type;
pub mod can_be_connected_to;
pub mod can_be_hydrated;
pub mod can_continue_sleeping;
pub mod can_continue_using;
pub mod can_drop_from_explosion;
pub mod can_entity_destroy;
pub mod can_equip;
pub mod can_harvest_block;
pub mod can_perform_action;
pub mod can_place_item;
pub mod can_player_sleep;
pub mod can_start_swimming;
pub mod can_sustain_plant;
pub mod chat_recipient;
pub mod chat_suggestion_applied;
pub mod chunk_ticket_level_updated;
pub mod chunk_watch;
pub mod client_attack_target;
pub mod client_block_changed;
pub mod client_chat;
pub mod client_chat_received;
pub mod client_chunk_load;
pub mod client_chunk_unload;
pub mod client_gui_state;
pub mod client_information_updated;
pub mod client_input_reads_tests;
pub mod client_mining_predict;
pub mod client_packet;
pub mod client_packet_send;
pub mod client_pause_change;
pub mod client_pause_read;
pub mod client_player_change_game_type;
pub mod client_player_clone;
pub mod client_player_logging_in;
pub mod client_player_logging_out;
pub mod client_services;
pub mod client_setup;
pub mod client_stopped;
pub mod client_stopping;
pub mod client_text;
pub mod client_tick_post;
pub mod client_tick_pre;
pub mod clone_item_stack;
pub mod component_hover;
pub mod compute_fov_modifier;
pub mod connects_to;
pub mod console_line;
pub mod contained_fluid;
pub mod container_loot_filled;
pub mod crafting_remainder;
pub mod create_fluid_source;
pub mod create_skull_models;
pub mod create_spawn_position;
pub mod creative_tabs_read;
pub mod creator_mod_id;
pub mod crop_grow;
pub mod custom_click_action;
pub mod custom_spawners;
pub mod customize_boss_event_progress;
pub mod customize_chat_overlay;
pub mod damage_item;
pub mod data_maps_updated;
pub mod default_components_bound;
pub mod destroy_progress;
pub mod dispense_method;
pub mod effect_particle_modification;
pub mod empty_contents;
pub mod enchant_power_bonus;
pub mod enchanted_block_loot;
pub mod enchanted_entity_loot;
pub mod enchantment_level_set;
pub mod enchantments;
pub mod enderman_anger;
pub mod entity_change_block_pre;
pub mod entity_constructing;
pub mod entity_entering_section;
pub mod entity_inside;
pub mod entity_interact_specific;
pub mod entity_invulnerability_check;
pub mod entity_mob_griefing;
pub mod entity_outline;
pub mod entity_place;
pub mod entity_size;
pub mod entity_struck_by_lightning;
pub mod entity_travel_to_dimension;
pub mod exp_drop;
pub mod explosion_detonate;
pub mod explosion_knockback;
pub mod explosion_notice_containment;
pub mod explosion_resistance;
pub mod explosion_start;
pub mod extend_poi_types;
pub mod extract_block_screen_effect;
pub mod falling_tick;
pub mod farmland_trample;
pub mod fence_gate_in_wall;
pub mod finalize_spawn;
pub mod fire_behaviour;
pub mod fire_odds;
pub mod fishing_medium;
pub mod flip_frame;
pub mod fluid_passage;
pub mod fluid_place_block;
pub mod fluid_rules;
pub mod fluid_tick;
pub mod fluid_tooltip;
pub mod friction;
pub mod game_rule_changed;
pub mod game_shutting_down;
pub mod gather_effect_screen_tooltips;
pub mod gather_skipped_attribute_tooltips;
#[cfg(test)]
pub mod general_entity_tick;
pub mod get_arm_pose;
pub mod grindstone_place;
pub mod grindstone_take;
pub mod gui_ticks;
pub mod held_item_tint;
pub mod highlight_tip;
pub mod humanoid_setup_anim;
pub mod ignited_by_lava;
pub mod incoming_tests;
pub mod ingredient_test;
pub mod input_key;
pub mod interaction;
pub mod is_fertile;
pub mod is_fire_source;
pub mod is_not_replaceable_by_pick_action;
pub mod is_pathfindable;
pub mod is_piglin_currency;
pub mod is_possible_to_respawn_in;
pub mod is_primary_item_for;
pub mod is_shearable;
pub mod is_valid_book_shelf;
pub mod item_after_vertical_collision;
pub mod item_attack;
pub mod item_container;
pub mod item_crafted;
pub mod item_destroyed;
pub mod item_fished;
pub mod item_interact_mob;
pub mod item_model_property;
pub mod item_picked_up;
pub mod item_smelted;
pub mod item_stacked_on_other;
pub mod item_tooltip;
pub mod item_toss;
pub mod key_mapping_pre;
pub mod left_click_empty;
pub mod level_save;
pub mod level_sounds;
pub mod level_tick_tests;
pub mod level_world;
pub mod light_emission;
pub mod living_breathe;
pub mod living_change_target;
pub mod living_conversion_post;
pub mod living_conversion_pre;
pub mod living_damage_post;
pub mod living_damage_pre;
pub mod living_death;
pub mod living_death_message;
pub mod living_death_post;
pub mod living_death_protection;
pub mod living_destroy_block;
pub mod living_drops;
pub mod living_experience_drop;
pub mod living_get_projectile;
pub mod living_shield_block;
pub mod living_use_totem;
pub mod living_visibility;
pub mod makes_piglins_neutral;
pub mod map_decoration_types;
pub mod math;
pub mod menu_extra_data;
pub mod menu_links;
pub mod menu_pick_all;
pub mod menu_slot_changed;
pub mod menu_slot_rules;
pub mod menu_slot_take;
pub mod menu_transfer;
pub mod minecart_pass;
pub mod mob_effect_events;
pub mod mob_spawn_checks;
pub mod mob_split;
pub mod mod_mismatch;
pub mod mod_settings;
pub mod model_baking_completed;
pub mod movement_input;
pub mod muffle_sounds;
pub mod must_survive;
pub mod native_note_block_change;
pub mod native_potential_spawns;
pub mod native_village_siege;
pub mod neighbor_notify;
pub mod note_block_play;
pub mod on_block_exploded;
pub mod on_block_placed_by;
pub mod on_block_state_change;
pub mod on_caught_fire;
pub mod on_chunk_unloaded;
pub mod on_destroyed_by_player;
pub mod on_destroyed_by_push_reaction;
pub mod on_dropped_by_player;
pub mod on_item_use_first;
pub mod on_sheared;
pub mod on_stop_using;
pub mod panel_recipes;
pub mod permissions_changed;
pub mod piston_carries_block_entity;
pub mod piston_push_reaction;
pub mod place_liquid;
pub mod placement_context;
pub mod play_hit_sound;
pub mod player_bed_leave;
pub mod player_break_speed;
pub mod player_brewed_potion;
pub mod player_destroy_item;
pub mod player_destroyed_block;
pub mod player_enchant_item;
pub mod player_flyable_fall;
pub mod player_game_mode_changed;
pub mod player_harvest_check;
pub mod player_heart_type;
pub mod player_inventory_changed;
pub mod player_item_smithed;
pub mod player_leave;
pub mod player_move;
pub mod player_passage_cross;
pub mod player_pong;
pub mod player_set_spawn;
pub mod player_spawn_phantoms;
pub mod player_start_spectating_entity;
pub mod player_swap_hands;
pub mod player_xp_change;
pub mod player_xp_level_change;
pub mod portal_spawn;
pub mod potion_brew_post;
pub mod potion_brew_pre;
pub mod pre_remove_side_effects;
pub mod rails;
pub mod recipe_book_categories;
pub mod recipe_book_ghost;
pub mod recipe_book_place;
pub mod recipe_sources;
pub mod register_block_state_models;
pub mod register_block_tint_sources;
pub mod register_cauldron_fluid_content;
pub mod register_cauldron_interaction;
pub mod register_color_resolvers;
pub mod register_debug_entries;
pub mod register_debug_renderers;
pub mod register_dimension_transition_screen;
pub mod register_entity_renderers;
pub mod register_entity_spectator_shaders;
pub mod register_fluid_models;
pub mod register_game_tests;
pub mod register_item_models;
pub mod register_item_tint_sources;
pub mod register_json_animation_types;
pub mod register_layer_definitions;
pub mod register_load_conditions;
pub mod register_model_loaders;
pub mod register_particle_groups;
pub mod register_particle_providers;
pub mod register_preset_editors;
pub mod register_range_select_item_model_property;
pub mod register_render_pipelines;
pub mod register_select_item_model_property;
pub mod register_special_model_renderer;
pub mod register_sprite_sources;
pub mod register_texture_atlases;
pub mod register_villager_professions;
pub mod registered_potions;
pub mod remove_block;
pub mod render_arm;
pub mod render_frame_post;
pub mod render_frame_pre;
pub mod render_gui_post;
pub mod render_gui_pre;
pub mod render_hand;
pub mod render_item_in_frame;
pub mod render_level_stage;
pub mod render_living_post;
pub mod render_living_pre;
pub mod render_player_post;
pub mod render_player_pre;
pub mod render_tooltip_gather_components;
pub mod render_tooltip_pre;
pub mod render_tooltip_texture;
pub mod resolve_player_skin;
pub mod respawn_position;
pub mod right_click_empty;
pub mod right_click_item;
pub mod runtime_environment;
pub mod screen_chars;
pub mod screen_closing;
pub mod screen_exclusion_areas;
pub mod screen_hovered_stack;
pub mod screen_init_post;
pub mod screen_input_post;
#[cfg(test)]
pub mod screen_input_tests;
pub mod screen_opening;
pub mod screen_preedit_effects;
pub mod screen_render_stages;
pub mod screen_room;
pub mod screen_transparent_background;
pub mod screenshot;
pub mod section_visibility;
pub mod server_chat;
pub mod server_tick_rate_changed;
pub mod set_placed_by;
pub mod should_cause_block_break_reset;
pub mod should_cause_reequip_animation;
pub mod should_check_weak_power;
pub mod should_display_fluid_overlay;
pub mod should_hide_adjacent_fluid_face;
pub mod should_redstone_wire_connect_to;
pub mod sound_and_shader_helpers;
pub mod sound_type;
pub mod spawner_active;
pub mod spread_players_teleport;
pub mod state_at_viewpoint;
pub mod stateful_block_entity;
pub mod submit_custom_geometry;
pub mod supports_enchantment;
pub mod survival_query_guard;
pub mod sweep_attack;
pub mod tags_updated;
pub mod teleport_command;
pub mod texture_atlas_stitched;
pub mod tick_target_millis;
pub mod toast_add;
pub mod tool_modified_state;
pub mod translate;
pub mod use_item_hand;
pub mod use_item_on_block;
pub mod vanilla_game_event;
pub mod viewed_level_sound;
pub mod wind_burst;
pub mod window_resize;
pub mod xp_orb_targeting;
pub mod xp_repair_ratio;
#[allow(unused_imports)]
pub use base::LevelView;
#[allow(unused_imports)]
pub use bounce_restitution::install_block_contact_hooks;
#[allow(unused_imports)]
pub use living_change_target::NativeHookHost;
pub use mod_api::ModBlockEntity;
pub use sound_and_shader_helpers::sound_id;
pub use survival_query_guard::enter_query as enter_survival_query;

// Free-function associated native hooks re-exported at native_hooks::*
#[allow(unused_imports)]
pub use add_section_geometry::native_add_section_geometry;
#[allow(unused_imports)]
pub use base::{
    native_block_entity_update_tag, native_blocks, native_component_types, native_items,
    native_menu_button, native_menu_quick_move, native_standalone_models,
};
#[allow(unused_imports)]
pub use block_entity_on_load::native_block_entity_on_load;
#[allow(unused_imports)]
pub use block_entity_on_load::{authoritative_world_for_mods, world_for_mods};
#[allow(unused_imports)]
pub use block_entity_type_add_blocks::native_block_entity_type_add_blocks;
#[allow(unused_imports)]
pub use can_place_item::native_can_place_item;
#[allow(unused_imports)]
pub use create_skull_models::native_create_skull_models;
#[allow(unused_imports)]
pub use extend_poi_types::native_extend_poi_types;
#[allow(unused_imports)]
pub use map_decoration_types::native_map_decoration_types;
#[allow(unused_imports)]
pub use math::startup_math_profile;
#[allow(unused_imports)]
pub use menu_slot_take::native_menu_slot_take;
#[allow(unused_imports)]
pub use piston_push_reaction::native_piston_push_reaction;
#[allow(unused_imports)]
pub use register_block_state_models::native_register_block_state_models;
#[allow(unused_imports)]
pub use register_block_tint_sources::native_register_block_tint_sources;
#[allow(unused_imports)]
pub use register_cauldron_fluid_content::native_register_cauldron_fluid_content;
#[allow(unused_imports)]
pub use register_cauldron_interaction::native_register_cauldron_interaction;
#[allow(unused_imports)]
pub use register_color_resolvers::native_register_color_resolvers;
#[allow(unused_imports)]
pub use register_debug_entries::native_register_debug_entries;
#[allow(unused_imports)]
pub use register_debug_renderers::native_register_debug_renderers;
#[allow(unused_imports)]
pub use register_dimension_transition_screen::native_register_dimension_transition_screen;
#[allow(unused_imports)]
pub use register_entity_renderers::native_register_entity_renderers;
#[allow(unused_imports)]
pub use register_entity_spectator_shaders::native_register_entity_spectator_shaders;
#[allow(unused_imports)]
pub use register_fluid_models::native_register_fluid_models;
#[allow(unused_imports)]
pub use register_item_models::native_register_item_models;
#[allow(unused_imports)]
pub use register_item_tint_sources::native_register_item_tint_sources;
#[allow(unused_imports)]
pub use register_json_animation_types::native_register_json_animation_types;
#[allow(unused_imports)]
pub use register_layer_definitions::native_register_layer_definitions;
#[allow(unused_imports)]
pub use register_load_conditions::native_register_load_conditions;
#[allow(unused_imports)]
pub use register_model_loaders::native_register_model_loaders;
#[allow(unused_imports)]
pub use register_particle_groups::native_register_particle_groups;
#[allow(unused_imports)]
pub use register_particle_providers::native_register_particle_providers;
#[allow(unused_imports)]
pub use register_preset_editors::native_register_preset_editors;
#[allow(unused_imports)]
pub use register_range_select_item_model_property::native_register_range_select_item_model_property;
#[allow(unused_imports)]
pub use register_render_pipelines::native_register_render_pipelines;
#[allow(unused_imports)]
pub use register_select_item_model_property::native_register_select_item_model_property;
#[allow(unused_imports)]
pub use register_special_model_renderer::native_register_special_model_renderer;
#[allow(unused_imports)]
pub use register_sprite_sources::native_register_sprite_sources;
#[allow(unused_imports)]
pub use register_texture_atlases::native_register_texture_atlases;
#[allow(unused_imports)]
pub use register_villager_professions::native_register_villager_professions;
#[allow(unused_imports)]
pub use system_helpers::mod_data_root;

include!("shared.rs");

use crate::game::native_hooks::base::GameHooks as _;

use crate::game::side_ops::modapi_hooks::ModapiHooksSideGameExt as _;

use crate::game::player_identity::PlayerIdentityGameExt1 as _;

mod client_callbacks;
pub use client_callbacks::install_client_hooks;
