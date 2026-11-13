# Upstream sync 31

Upstream pin: `a21f8a3947e9fa6fdf47cbc0f656d6444a622e2a`, the newest completed-success push-to-main `ci.yml` head at selection ([run 43192928360](https://github.com/horizon-rust/horizon/actions/runs/43192928360), completed 2026-11-12 09:52:42 UTC). Previous integrated upstream: `911aa6ddc6ec1684eaa312c7650e938789cf2193`. Pin is fixed during integration.

Started from fork main `503cdc1ffa63c38876f769a6cb6253e23313b62a`. Incoming delta: 59 paths, with 23 initial conflicts. Fresh main was fetched and merge attempted once immediately before final compile-only verification; it remained `503cdc1ffa63c38876f769a6cb6253e23313b62a` (already up to date). Final scope: 66 files.

## Production ownership

- Keep hook-aware orb ticking and the lending LoopContext. Movement returns its landing event to ordered ticking, retaining powder-snow contact, landing vibration, water reset, lava timing and lifetime/attachment ownership. Cloud/orb/trident fall distance persists through actual model codecs and command mutation paths.
- Use one player wind-charge no-deflect countdown, initialized on the actual model, checked by production deflection and decremented by ticking. Do not replace the typed projectile ownership/hook path.
- Invalid guest attacks use resident Target resolution and existing authoritative physical bounds, reach, border, piercing-weapon and redirectable-projectile tag rules. The check runs at the actual guest packet boundary before attack hooks; it does not scan duplicate raw lists.
- Synced nameplate attributes update local player, mob and armor-stand owners. Rendering and below-name visibility use those values; typed observe-client-packet hooks, identity and residency remain intact.
- Shared prepared vanilla fixtures are test support, cached per process while preserving missing-resource attribution. Production resource/mod roots remain authoritative and are not replaced by fixture defaults.
- Take upstream's one-peak HDR mastering and gains across actual terrain, entity, custom-sky, OIT, star and post-processing writers. Keep fork view_origin uniform layout, precision tolerances and the existing narrow software-adapter skip unchanged. No HDR measurement claim is made.
- Process probes and the byte-identical scrubber move to horizon-system; Horizon-layer re-exports remain as upstream ships them. Dedicated performance reporting uses system probes off the tick. The client already depended on horizon-system, so discard the silently duplicated auto-merge manifest key. OptiFine's dev-only probe dependency changes from bench to system while retaining fork asset/zip dependencies.
- Take the orb fall-distance command corpus and real dedicated-server wind-charge parity harness. Neither was run locally.

## Fork behavior kept at the new guest attack gate

| Boundary | Decision |
| --- | --- |
| Placed fake player | Accept a Fake guest placed in the hub's active dimension as well as an online guest. Keep participates_online network-only. Fake PlayerAction::Attack must still reach the native attack_entity hook and Player.attack. The existing registered fake-attack fixture now explicitly asserts fake/placed/current-dimension/non-online setup before its unchanged hook, damage and cancellation assertions. |
| Guest already leaving | Stop when left_reason is nonempty. Upstream filters new NetEvent packets before putting them in an inbox, but neither the attack dispatcher nor the new gate rechecks a kick after events have been drained as a batch. This one-line boundary guard prevents later attacks in that batch. |
| Horizon-layer probes | Keep upstream's horizon_bench process-probe re-export and current callers: restoring upstream's Horizon layer takes precedence over treating this as a mod API compatibility path. |
| Nameplate range fixture | Keep upstream's existing origin-centered box assertion unchanged in this repair. Its distance-isolation weakness is a coverage carry, not evidence that the new distance behavior ran. |

Sync checklist: for every newly imported Guest gate, check each fork-only GuestKind, especially Fake, through its actual action boundary rather than assuming online participation and simulation actions are the same predicate.

## Dead incoming files and deliberate divergences

Diverges from upstream: incoming entity_nbt/orb.rs, game/item_entities.rs, game/loaded_arrow_shot_tests.rs, game/lan_actions.rs and game/remote_attributes.rs are not compiled owners on the fork. Their changed behavior maps to the inline typed codecs, game orb creation, creatures fixture module, network/lan_actions and network/remote owners respectively. Three new resource-backed rocket/orb tests and one missing sound de-duplication test are restored at those actual compiled owners.

The incoming world-warp fragment shader has no compiled Rust owner. Its HDR writer behavior maps to the compiled terrain/entity/OIT writers; the dead source-level adapter is not revived.

Whole-file incoming inventory: **604 declarations across 36 changed test-bearing paths**. Entity area accounts for 320, network for 136, render for 114 and system/bench/telemetry/report for 34. Six incoming tests are not on this tree and are not reproduced verbatim:

- `reasons_round_trip`: the old Reason::ALL/string parser API is absent; the typed SpawnReason contract is authoritative. No parser shim or trivial derive test replaces it.
- `find_join_finds_by_number_in_or_out_of_order`: the raw find_join helper is gone; typed JoinList owns lookup and the retained first_join reorder test exercises production ordering.
- `horizon_channels_reach_their_handlers_and_others_go_to_the_mods`: the old string classifier is absent; current typed payload admission/dispatch is authoritative.
- `actual_guest_input_model_pose_reaches_self_metadata`: its externally owned mutable-host setup is absent. Current publisher coverage decodes actual self metadata through GuestList updates, but is not equivalent evidence for the old model-input integration setup.
- `a_crossbow_rocket_posts_on_its_first_tick_and_a_mob_trident_at_once`: not present on the fork's tree; its rocket-plus-mob-trident first-tick pairing is not covered by another test here.
- `place_block_alias_survival_consumption_and_client_prediction`: not present on the fork's tree; the alias placement's survival consumption with client prediction is not covered by another test here.

These are explicit existing API divergences, not passing tests or exhaustive equivalence claims. The remaining mappings include inherited unchanged declarations, renamed tests and compiled extracted test modules.

## Resource-gated incoming tests

A declaration that returns early or is ignored is not execution evidence. The lists below name all incoming inventory declarations identified as gated. Prepared fixtures require movement-properties and entity-type data; game integration fixtures use prepared HORIZON_TEST_RESOURCES and isolated homes as their helper requires. GPU fixtures depend on adapter/device requests and the listed capability gates. OS-conditioned machine probes are not vanilla-resource gates.

The new wind-charge dedicated-server parity harness additionally requires prepared vanilla/server inputs and a generated world through locator.sides. Sync 30's fox defense and village-shelter assertions are not new incoming declarations here, but remain resource-gated; their previous green CI returned early without vanilla data. No local resource-backed assertions are claimed run in this sync.

### Named gated declarations

#### `crates/horizon-client-render/src/gfx.rs`

- `the_16_bit_frame_needs_every_usage_and_flag_the_passes_take`
- `real_rail_mips_do_not_change_with_larger_array_layers`

#### `crates/horizon-client-render/src/gfx/custom_sky.rs`

- `written_custom_sky_modes_keep_factors_and_scale_only_screen_source`

#### `crates/horizon-client-render/src/gfx/hdr.rs`

- `shared_light_matches_the_original_for_every_mark`
- `coverage_target_resizes_resets_and_shows_in_the_debug_view`
- `direct_light_matches_the_legacy_non_float_display_below_the_tone_knee`
- `the_writers_light_is_the_same_on_every_display`
- `the_tone_curve_maps_the_scene_peak_onto_the_display`
- `a_switched_encoding_keeps_the_tuning_and_shows_the_same_light`
- `the_black_level_lift_follows_bt2390_in_pq_and_scrgb`
- `a_scaled_screenshot_shoots_at_its_size_and_leaves_the_window_its_own_targets`
- `sdr_screenshots_roll_off_the_world_and_keep_the_hud_as_drawn`
- `the_hud_lays_over_the_worlds_light_by_its_coverage`
- `the_hud_at_ui_brightness_stops_at_the_peak`
- `the_menu_blur_keeps_the_light_it_blurs`

#### `crates/horizon-client-render/src/gfx/hdr_written_tests.rs`

- `float_plain_fragments_keep_rgb_and_write_only_coverage`
- `float_stars_keep_the_original_rgb_with_brightness_as_coverage`
- `float_material_wrappers_keep_rgb_and_coverage_including_displacement`
- `actual_opaque_writers_match_the_legacy_non_float_oracle`
- `actual_translucent_writers_blend_their_own_light`
- `unmarked_entity_users_keep_their_source_gain`
- `additive_texels_below_the_mark_cutoff_keep_their_gain`
- `actual_oit_terrain_writes_its_own_light_including_clear_glass`
- `actual_oit_particle_writes_its_own_light`
- `actual_oit_cloud_writes_its_own_light`
- `actual_oit_additive_adds_converted_light_without_coverage`

#### `crates/horizon-client-render/src/gfx/native_post.rs`

- `a_scripted_clock_drives_time_frame_index_and_eased_values`
- `a_reset_frame_gives_a_fresh_pack_s_inputs`
- `a_reset_post_reads_its_own_targets_blank`
- `passes_chain_through_half_float_targets_with_options_and_depth`
- `the_last_pass_reads_last_frames_input_as_history`
- `the_surface_is_an_integer_target_that_the_post_stage_reads_bit_exact`
- `the_surface_keeps_every_bit_of_the_floats_written_through_the_material_stage_bitcast`
- `a_0_2_world_comes_back_through_decode_tonemap_and_encode`
- `a_0_2_frame_describes_the_display_it_goes_to`
- `a_0_2_pack_is_tone_mapped_once_by_its_own_curve_or_horizons`

#### `crates/horizon-client-render/src/gfx/world_post.rs`

- `culling_uses_the_next_taa_size_before_the_first_jitter_or_resize`
- `baked_light_changes_and_returning_from_a_pack_reject_taa_history`
- `world_post_pipelines_compile_or_fall_back`

#### `crates/horizon-client/src/entities.rs`

- `ordinary_items_burn_but_native_netherite_resists_lava` — requires items.json; missing input is attributed to the calling test.
- `lava_that_destroys_a_bundle_spills_its_contents` — requires items.json; missing input is attributed to the calling test.
- `a_dropped_item_on_soul_sand_or_honey_is_slowed_by_the_move`
- `the_world_border_stops_items_and_experience_orbs`
- `a_resting_item_skips_move_three_of_four_ticks`
- `a_grounded_item_in_a_soul_sand_column_rises`
- `an_item_at_the_top_of_a_bubble_column_is_at_its_surface`
- `a_slime_bounce_marks_the_item_for_immediate_sync`
- `an_item_lands_on_an_entity_box_under_it`
- `a_cobweb_and_powder_snow_slow_an_item`
- `obstacle_grid_beats_scanning_every_box_for_a_big_farm`
- `honey_slide_runs_after_stand_travel_and_preserves_no_physics_controls`
- `climbing_uses_actual_blocks_and_post_collision_position`
- `the_world_border_stops_an_armor_stand`
- `a_stand_lands_on_and_stops_at_the_entity_boxes_it_is_given`
- `a_tick_keeps_the_previous_position_for_interpolation`
- `a_no_gravity_stand_stays_put`
- `leather_boots_walk_on_powder_snow`
- `a_burning_item_is_drawn_at_full_block_light`
- `a_server_orb_takes_lava_damage_after_moving_and_discards_at_zero`
- `legacy_orb_in_air_or_water_or_non_lava_does_not_play_sound_or_consume_random`
- `negative_case_non_contact_or_client_orb_does_not_play_burn_sound`
- `positive_case_server_orb_in_lava_plays_generic_burn_sound`
- `boundary_case_surviving_and_discarding_and_multi_contact_in_one_tick`

#### `crates/horizon-client/src/entities/fluids.rs`

- `fluid_flow_runs_away_from_a_pool_and_is_zero_in_still_water`
- `items_follow_flowing_water_but_still_water_has_no_horizontal_push`
- `experience_orbs_follow_flowing_water_but_still_water_has_no_horizontal_push`
- `an_orb_whose_eye_dips_into_water_floats_with_its_feet_in_air`
- `a_boat_box_is_pushed_by_the_current_it_sits_in`
- `armor_stands_follow_flowing_water_but_markers_and_still_water_stay_in_place`

#### `crates/horizon-client/src/entity_nbt.rs`

- `loaded_item_lifetime_counters_tick_with_signed_sentinels`

#### `crates/horizon-client/src/game/commands.rs`

- `teleport_and_rotate_resolve_pitch_at_their_own_boundary`
- `a_vehicle_carries_its_riders_and_a_rider_leaves_its_vehicle`
- `actual_selector_uses_body_overlap_but_distance_stays_at_the_origin`
- `h149_actual_registered_local_commands_preserve_exact_saved_positions`
- `a_silent_source_drops_query_results_but_still_stores_them`
- `registered_presence_uses_actual_catalog_and_stack_patch`
- `ordinary_fill_normalizes_a_note_block_against_real_stone`
- `an_existing_default_state_does_not_skip_note_normalization`
- `strict_explicit_and_filtered_out_inputs_keep_their_existing_contracts`
- `actual_chest_damage_range_uses_live_damage_and_effective_presence`
- `kill_spills_explicit_vehicle_contents_after_menu_writeback`
- `actual_registered_global_selector_keeps_interleaved_insertion_and_limits`

#### `crates/horizon-client/src/game/commands/data.rs`

- `saved_orb_uuid_comes_from_registered_identity_without_or_despite_raw`
- `live_wolf_anger_writes_use_the_level_clock_and_keep_bee_state`
- `actual_boat_getter_projects_runtime_children_and_existing_bookkeeping`
- `data_commands_on_an_unopened_loaded_chest_survive_a_save_and_reload`
- `loaded_arrow_getter_uses_registered_owner_reference_without_raw`
- `admitted_modern_literals_reach_actual_storage_as_typed_tags`
- `placed_bell_is_a_real_stored_entity_with_command_metadata`
- `commands_mutate_the_resident_falling_model_without_rejoining`
- `loaded_vehicle_saves_own_only_their_container_fields`

#### `crates/horizon-client/src/game/item_entities.rs`

- `spilled_stacks_rise_from_their_own_randoms`
- `a_damaged_shulker_box_spills_its_stacks_upward`
- `an_item_escapes_a_block_at_its_own_randoms_speed`

#### `crates/horizon-client/src/game/lan_actions.rs`

- `only_a_server_only_use_swings_back_to_the_player`
- `ordinary_guest_final_close_posts_to_actual_listener`
- `failed_stick_use_keeps_group_available_for_placement`
- `actual_guest_placement_starts_its_cooldown`
- `actual_guest_second_placement_is_rejected`

#### `crates/horizon-client/src/game/lan_actions/attack_checks.rs`

- `attacking_an_item_an_orb_oneself_or_an_arrow_kicks_the_player`

#### `crates/horizon-client/src/game/loaded_arrow_shot_tests.rs`

- `a_rocket_posts_its_shot_once_and_a_silent_one_launches_quietly`
- `a_landing_orb_posts_hit_ground_after_a_fall`
- `a_falling_rocket_hits_the_ground_and_one_in_water_splashes`
- `loaded_arrow_first_tick_dispatches_once_with_its_real_owner`
- `bow_launch_uses_only_the_first_tick_event`
- `crossbow_launch_uses_only_the_first_tick_event`
- `mob_launch_uses_only_the_first_tick_event`
- `finish_hit_dispatches_once_and_the_next_tick_does_not_repeat_it`
- `finish_hit_uses_registered_guest_context_and_restores_the_host`

#### `crates/horizon-client/src/game/names.rs`

- `a_mobs_name_is_sent_with_its_style`

#### `crates/horizon-client/src/game/perspective.rs`

- `player_render_transform_uses_vanilla_avatar_scale`
- `an_invisible_body_keeps_its_armour`

#### `crates/horizon-client/src/game/remote.rs`

- `joining_a_server_empties_the_off_hand_too`
- `tracking_emitters_follow_the_packet_target`
- `a_servers_client_takes_its_speed_from_the_attributes_not_the_effect`
- `mod_open_chat_fills_the_box_only_over_the_world`
- `plugin_player_stack_follows_network_moves_and_local_carriers`
- `mod_chat_needs_a_joined_player_and_what_the_chat_option_allows`
- `a_reconfiguring_client_plays_menu_music_and_never_indexes_past_the_biomes`
- `a_predicted_throw_from_a_slot_swings_the_hand`
- `placement_preview_checks_obstruction_without_changing_the_level`
- `a_rocket_used_while_gliding_ends_the_click`
- `flint_and_steel_use_on_ends_the_click`

#### `crates/horizon-client/src/game/trident.rs`

- `trident_release_rechecks_stack_durability_after_charge`

#### `crates/horizon-client/src/lan/prediction.rs`

- `a_client_on_its_loading_screen_is_over_without_a_move`
- `item_uses_a_few_ticks_apart_from_the_servers_are_played`
- `a_use_without_its_slowdown_is_caught`
- `a_played_tick_tells_its_offset_and_state`
- `a_cheats_worst_tick_is_kept_with_its_offset`
- `an_honest_client_is_played_again_exactly`
- `a_faster_or_flying_client_is_not`
- `attributes_sent_mid_sprint_are_played_as_the_client_takes_them`
- `a_push_is_played_on_the_tick_the_client_took_it`
- `a_cheat_counts_from_where_it_started`
- `a_burst_keeps_the_setback_before_its_lie`
- `an_excused_push_is_caught_up_with`
- `a_missed_push_counts_once`
- `a_small_speed_lie_kept_up_still_counts`
- `a_sprint_the_server_cleared_is_started_again`
- `a_speed_lie_through_a_sprint_restart_still_counts`
- `a_sprint_the_server_stopped_after_a_hit_is_the_clients`
- `a_sprint_kept_without_the_key_or_sped_up_after_a_hit_counts`
- `a_glide_is_played_again_exactly`
- `a_burst_before_a_teleport_is_played_first`
- `a_step_is_caught`
- `a_spider_is_caught`
- `a_jesus_is_caught`

#### `crates/horizon-client/src/mobs/world.rs`

- `scaffolding_blocks_sight_only_from_above`


## Verification

The landing-orb unit fixture injects OrbMove and on_ground state: it exercises production landing-event bookkeeping, not the complete collision/fluid-to-landing path. The separate command corpus drives real motion, but was not run locally. Neither fixture presence nor resource-gated early return is runtime evidence.

Rustfmt and compile-only `cargo check --workspace --all-targets` are the allowed local checks. Initial compilation identified erroneous missing-resource attribution on ordinary production branches and duplicate discovery of the new attack-check module. Both were corrected at their owners. Subsequent test compilation exposed stale NBT/Joined/Reason API fixtures and the complete orb-constructor field family; those are corrected without changing assertions or restoring legacy APIs. No local tests, clippy, harnesses or gameplay runs. Independent static review found no remaining concrete blocker after correction passes. The complete integrated `cargo check --workspace --all-targets` passed with zero warnings before the first push. Touched Rust was formatted, whitespace checks passed and the generated baseline passed the static size check. Exact-head CI and final root glance remain pending.
