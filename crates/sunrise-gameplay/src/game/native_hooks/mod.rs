#[cfg(test)]
pub(in crate::game) mod item_picked_up_tests;
#[cfg(any(test, feature = "test-support"))]
pub mod on_block_placed_by_tests;
pub mod xp_repair_ratio;

pub mod xp_orb_targeting;

pub mod wind_burst;

pub mod vanilla_game_event;

pub mod teleport_command;

pub mod spread_players_teleport;

pub mod set_placed_by;

#[path = "cut_merged_server_tick_rate_changed.rs"]
pub mod server_tick_rate_changed;

pub mod server_chat;

pub mod respawn_position;

pub mod register_game_tests;

pub(in crate::game) mod register_configuration_tasks;

pub mod recipe_sources;

pub mod recipe_book_place;

pub mod potion_brew_pre;

pub mod potion_brew_post;

pub mod player_xp_level_change;

pub mod player_xp_change;

pub mod player_swap_hands;

pub mod player_start_spectating_entity;

pub mod player_spawn_phantoms;

pub mod player_set_spawn;

pub mod player_pong;

pub mod player_passage_cross;

pub mod player_move;

pub mod player_item_smithed;

pub mod player_inventory_changed;

pub mod player_game_mode_changed;

pub mod player_flyable_fall;

pub mod player_enchant_item;

pub mod player_brewed_potion;

pub mod permissions_changed;

pub mod on_dropped_by_player;

pub mod on_destroyed_by_player;

pub mod on_block_placed_by;

pub mod on_block_exploded;

pub mod native_village_siege;

pub mod native_potential_spawns;

pub mod mob_split;

pub mod mob_spawn_checks;

pub mod mob_effect_events;

pub mod minecart_pass;

pub mod menu_transfer;

pub mod menu_slot_changed;

pub mod menu_links;

pub mod menu_extra_data;

pub mod makes_piglins_neutral;

pub mod living_visibility;

pub mod living_experience_drop;

pub mod living_drops;

pub mod living_destroy_block;

#[path = "cut_merged_living_death_post.rs"]
pub mod living_death_post;

pub mod living_death_message;

pub mod living_conversion_pre;

pub mod living_conversion_post;

#[path = "cut_merged_living_breathe.rs"]
pub mod living_breathe;

pub mod level_save;

#[path = "cut_merged_item_toss.rs"]
pub mod item_toss;

#[path = "cut_merged_item_picked_up.rs"]
pub mod item_picked_up;

pub mod item_fished;

pub mod item_destroyed;

pub mod item_crafted;

pub mod is_primary_item_for;

pub mod is_possible_to_respawn_in;

pub mod is_pathfindable;

pub mod is_not_replaceable_by_pick_action;

pub mod grindstone_take;

pub mod grindstone_place;

pub mod game_rule_changed;

pub mod fire;

pub mod finalize_spawn;

#[path = "cut_merged_farmland_trample.rs"]
pub mod farmland_trample;

pub mod falling_tick;

pub mod explosion_start;

pub mod explosion_knockback;

pub mod explosion_detonate;

pub mod entity_travel_to_dimension;

pub mod entity_struck_by_lightning;

pub mod entity_place;

pub mod entity_mob_griefing;

pub mod entity_interact_specific;

#[path = "cut_merged_entity_inside.rs"]
pub mod entity_inside;

pub mod entity_change_block_pre;

pub mod enderman_anger;

pub mod enchantment_level_set;

pub mod enchanted_entity_loot;

pub mod empty_contents;

pub mod effect_particle_modification;

pub mod dispense_method;

#[path = "cut_merged_damage_item.rs"]
pub mod damage_item;

pub mod custom_spawners;

pub mod custom_click_action;

pub mod create_spawn_position;

pub mod crafting_remainder;

pub mod console_line;

pub mod clone_item_stack;

#[path = "cut_merged_client_information_updated.rs"]
pub mod client_information_updated;

pub mod chunk_watch;

pub mod chunk_ticket_level_updated;

pub mod chat_recipient;

pub mod can_player_sleep;

pub mod can_entity_destroy;

pub mod can_continue_sleeping;

pub mod block_use_without_item;

pub mod baby_entity_spawn;

pub mod arrow_nock;

pub mod armor_hurt;

pub mod apply_enchantments;

pub mod animal_tame;

pub mod adjacent_block_path_type;

pub mod add_server_reload_listeners;

pub mod anvil_update_server;
pub mod dragon_fight;
pub mod gaze_disguise;
pub mod map_color;
pub mod menu_removed;
pub mod server_level_event;
#[cfg(test)]
mod server_level_event_tests;
pub mod sleep_status_announce;
pub mod sleepers_needed;
pub mod spawn_cluster_size;

include!("mod_server.rs");
