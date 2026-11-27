#[cfg(test)]
pub(crate) mod animate_tick_tests;
#[cfg(test)]
pub(crate) mod block_entity_client_tick_tests;
#[cfg(test)]
pub(crate) mod block_entity_neighbors_tests;
#[cfg(test)]
pub(crate) mod can_be_connected_to_tests;
#[cfg(test)]
pub(crate) mod client_chunk_hooks_tests;
#[cfg(feature = "client")]
#[cfg(test)]
pub(crate) mod client_pick;
#[cfg(test)]
pub(crate) mod creative_tabs_read_tests;
#[cfg(test)]
pub(crate) mod death_source_tests;
#[cfg(test)]
pub(crate) mod enchantment_and_lang_tests;
#[cfg(test)]
pub(crate) mod fence_gate_in_wall_tests;
#[cfg(test)]
pub(crate) mod fluid_draw;
#[cfg(test)]
pub(crate) mod guest_death_lifecycle_tests;
#[cfg(test)]
pub(crate) mod on_block_state_change_tests;
#[cfg(test)]
pub(crate) mod on_chunk_unloaded_tests;
#[cfg(test)]
pub(crate) mod on_destroyed_by_push_reaction_tests;
#[cfg(test)]
pub(crate) mod on_entity_item_update;
#[cfg(test)]
pub(crate) mod piston_push_reaction_tests;
#[cfg(test)]
pub(crate) mod placement_context_tests;
#[cfg(test)]
pub(crate) mod play_hit_sound_tests;
#[cfg(test)]
pub(crate) mod registered_potions_tests;
#[cfg(test)]
pub(crate) mod remove_block_tests;
pub(crate) mod tick_target_millis;
#[cfg(test)]
pub(crate) mod trident_family_tests;

include!("shared.rs");

pub(crate) mod server_tick_rate_changed;

pub(crate) mod runtime_environment;

pub(crate) mod render_tooltip_texture;

pub(crate) mod render_gui_post;

pub(crate) mod render_frame_pre;

pub(crate) mod register_preset_editors;

pub(crate) mod player_leave;

pub(crate) mod player_heart_type;

pub(crate) mod player_game_mode_changed;

pub(crate) mod player_bed_leave;

pub(crate) mod mod_mismatch;

pub(crate) mod math;

pub(crate) mod living_death_post;

pub(crate) mod level_tick_tests;

pub(crate) mod item_toss;

pub(crate) mod item_destroyed;

pub(crate) mod ingredient_test;

pub(crate) mod fluid_tick;

pub(crate) mod flip_frame;

pub(crate) mod farmland_trample;

pub(crate) mod entity_place;

pub(crate) mod entity_interact_specific;

pub(crate) mod entity_inside;

pub(crate) mod customize_boss_event_progress;

pub(crate) mod component_hover;

pub(crate) mod client_visual_text;

pub(crate) mod client_text;

pub(crate) mod client_packet_send;

pub(crate) mod client_information_updated;

pub(crate) mod build_creative_mode_tab_contents;

pub(crate) mod block_landing;

pub(crate) mod block_entity_save_additional_tests;

pub(crate) mod block_entity_on_load_tests;

pub(crate) mod block_entity_on_load_routes_tests;

#[cfg(all(test, feature = "client"))]
pub(crate) mod base;

#[cfg(test)]
mod mod_block_entity_payload_tests;

#[cfg(all(test, feature = "client"))]
pub(crate) mod play_break_sound_tests;
