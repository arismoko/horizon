#[cfg(feature = "client")]
use super::native_block_entity_draw::BlockEntityDraw;
pub(in crate::game) mod client_narrator;
#[cfg(test)]
pub(in crate::game) mod client_narrator_tests;
#[path = "cut_merged_client_options.rs"]
pub(in crate::game) mod client_options;
#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(in crate::game) use add_attribute_tooltips::HookedAttributeTooltips;
pub(in crate::game) mod window_resize;

pub(in crate::game) mod viewed_level_sound;

pub(in crate::game) mod toast_add;

pub(in crate::game) mod tick_target_millis;

pub(in crate::game) mod texture_atlas_stitched;

#[path = "cut_merged_substitute_inline_images.rs"]
pub(in crate::game) mod substitute_inline_images;

pub(in crate::game) mod submit_custom_geometry;

pub(in crate::game) mod state_at_viewpoint;

pub(in crate::game) mod should_hide_adjacent_fluid_face;

pub(in crate::game) mod should_display_fluid_overlay;

pub(in crate::game) mod should_cause_reequip_animation;

pub(in crate::game) mod should_cause_block_break_reset;

include!("shared.rs");

pub(in crate::game) mod section_visibility;

pub(in crate::game) mod screenshot;

#[path = "cut_merged_screen_transparent_background.rs"]
pub(in crate::game) mod screen_transparent_background;

pub(in crate::game) mod screen_room;

pub(in crate::game) mod screen_render_stages;

pub(in crate::game) mod screen_preedit_effects;

pub(in crate::game) mod screen_opening;

pub(in crate::game) mod screen_input_post;

pub(in crate::game) mod screen_init_post;

#[path = "cut_merged_screen_exclusion_areas.rs"]
pub(in crate::game) mod screen_exclusion_areas;

pub(in crate::game) mod screen_closing;

pub(in crate::game) mod right_click_empty;

#[path = "cut_merged_resolve_player_skin.rs"]
pub(in crate::game) mod resolve_player_skin;

pub(in crate::game) mod render_tooltip_texture;

pub(in crate::game) mod render_tooltip_pre;

pub(in crate::game) mod render_tooltip_gather_components;

pub(in crate::game) mod render_player_pre;

pub(in crate::game) mod render_player_post;

#[cfg(feature = "client")]
pub mod render_name_tag_can_render;

pub(in crate::game) mod render_living_pre;

pub(in crate::game) mod render_living_post;

pub(in crate::game) mod render_level_stage;

pub(in crate::game) mod render_item_in_frame;

pub(in crate::game) mod render_hand;

#[path = "cut_merged_render_gui_pre.rs"]
pub(in crate::game) mod render_gui_pre;

pub(in crate::game) mod render_gui_post;

#[path = "cut_merged_render_frame_pre.rs"]
pub(in crate::game) mod render_frame_pre;

#[path = "cut_merged_render_frame_post.rs"]
pub(in crate::game) mod render_frame_post;

pub(in crate::game) mod render_arm;

pub(in crate::game) mod recipe_book_ghost;

pub(in crate::game) mod player_heart_type;

#[path = "cut_merged_play_hit_sound.rs"]
pub(in crate::game) mod play_hit_sound;

pub(in crate::game) mod movement_input;

pub(in crate::game) mod model_baking_completed;

pub(in crate::game) mod left_click_empty;

pub(in crate::game) mod key_mapping_pre;

pub(in crate::game) mod item_tooltip;

pub(in crate::game) mod item_model_property;

pub(in crate::game) mod interaction;

pub(in crate::game) mod input_key;

pub(in crate::game) mod humanoid_setup_anim;

pub(in crate::game) mod hotbar_attack_indicator_progress;

pub(in crate::game) mod highlight_tip;

pub(in crate::game) mod held_item_tint;

pub(in crate::game) mod get_arm_pose;

pub(in crate::game) mod gather_skipped_attribute_tooltips;

pub(in crate::game) mod gather_effect_screen_tooltips;

pub(in crate::game) mod fluid_tooltip;

pub(in crate::game) mod flip_frame;

pub(in crate::game) mod fire;
pub(in crate::game) use fire::HotbarAttackIndicatorProgressCall;

pub(in crate::game) mod extract_block_screen_effect;

pub(in crate::game) mod entity_outline;

#[path = "cut_merged_customize_chat_overlay.rs"]
pub mod customize_chat_overlay;

#[path = "cut_merged_customize_boss_event_progress.rs"]
pub mod customize_boss_event_progress;

pub(in crate::game) mod contained_fluid;

pub(in crate::game) mod compute_fov_modifier;

pub(in crate::game) mod component_hover;

pub(in crate::game) mod client_tick_pre;

pub(in crate::game) mod client_tick_post;

pub(in crate::game) mod client_stopping;

pub(in crate::game) mod client_stopped;

pub(in crate::game) mod client_setup;

pub(in crate::game) mod client_services;

pub(in crate::game) mod client_player_logging_out;

pub(in crate::game) mod client_player_logging_in;

pub(in crate::game) mod client_player_clone;

pub(in crate::game) mod client_player_change_game_type;

pub(in crate::game) mod client_pause_change;

pub(in crate::game) mod client_packet_send;

pub(in crate::game) mod client_packet;

pub(in crate::game) mod client_mining_predict;

pub(in crate::game) mod client_chunk_unload;

pub(in crate::game) mod client_chunk_load;

pub(in crate::game) mod client_chat_received;

pub(in crate::game) mod client_chat;

pub(in crate::game) mod client_block_changed;

pub(in crate::game) mod client_attack_target;

pub(in crate::game) mod chat_suggestion_applied;

#[path = "cut_merged_camera_type.rs"]
pub(in crate::game) mod camera_type;

pub(in crate::game) mod camera_submersion;

pub(in crate::game) mod calculate_player_turn;

pub(in crate::game) mod break_progress_positions;

pub(in crate::game) mod block_render_visibility;

pub(in crate::game) mod block_particle_effects;

pub(in crate::game) mod block_entity_view_distance;

pub(in crate::game) mod arm_pose_transform;

#[path = "cut_merged_animate_tick.rs"]
pub(in crate::game) mod animate_tick;

pub(in crate::game) mod add_client_reload_listeners;

pub(in crate::game) mod add_attribute_tooltips;

#[cfg(feature = "client")]
include!("mod_client.rs");

#[cfg(all(test, feature = "client"))]
mod screen_slot_tests;

#[cfg(all(test, feature = "client"))]
mod panel_recipes_tests;

#[cfg(feature = "client")]
pub mod anvil_update_client;
#[cfg(feature = "client")]
pub mod camera_position;
#[cfg(feature = "client")]
pub mod client_break_progress;
#[cfg(feature = "client")]
pub mod client_particle_add;
#[cfg(feature = "client")]
pub mod client_take_item_entity;
#[cfg(all(test, feature = "client"))]
mod client_take_item_entity_tests;
#[cfg(feature = "client")]
pub mod compute_camera_angles;
#[cfg(feature = "client")]
pub mod frame_graph_setup;
#[cfg(all(test, feature = "client"))]
mod framerate_limit_tests;
#[cfg(feature = "client")]
pub(in crate::game) mod gui_status_heights;
#[cfg(all(test, feature = "client"))]
mod mod_settings_tests;
#[cfg(feature = "client")]
pub mod play_break_sound;
#[cfg(feature = "client")]
pub(in crate::game) mod play_sound;
#[cfg(feature = "client")]
pub mod player_turn;
#[cfg(feature = "client")]
pub mod prepare_render_buffers;
#[cfg(feature = "client")]
pub mod register_pipeline_modifiers;
#[cfg(feature = "client")]
pub use register_pipeline_modifiers::native_register_pipeline_modifiers;
#[cfg(feature = "client")]
pub mod render_name_tag_do_render;
#[cfg(all(test, feature = "client"))]
mod render_name_tag_do_render_tests;
#[cfg(feature = "client")]
pub mod render_view_transform;
#[cfg(feature = "client")]
pub mod tick_weather_effects;
#[cfg(all(test, feature = "client"))]
mod tick_weather_effects_tests;
