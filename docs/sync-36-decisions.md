# Sync 36 decisions

## Pins and scope

Integrate upstream `441f49d57e344bd56b90fd4af5c4e443ff3937c9` directly from fork main `b8af9dedf5adcb752661016d169141e4641c5c57`. Upstream's own main-push checks passed in run 43393547822. The integration includes upstream's Horizon layer and older-client protocol, graphics, upload pacing, item-use and gameplay fixes. The target is not chased after the first push.

## Conflict decisions

- Preserve the fork's generated logical game-module discovery and topic folders. Locate upstream destinations by filename, taking both halves of moves rather than retaining duplicate root modules. The new upload-pacing module belongs under `game/interface/` with the same logical module name.
- Use upstream's single non-optional `UseStart` record, carrying its captured optional stack and tick-override flag. Remove the duplicate fork fields. Preserve hook-aware start, tick, stop and release behavior, item replacement, and guest save/restore through that record; the separate remote-client field is unrelated.
- Preserve typed hook dispatch, Host ownership, lifetime-aware admission reasons, prepared item state and entity-owned random streams. The default mob loop remains borrowed; no optional parallel feature is activated.
- Keep the fork's canonical `explosion_detonate` moment rather than adding `explosion_entities`. Existing item-damage transactions implement upstream's damage-resistance behavior without a second damage path.
- Keep the maintainer-approved `MAX_CHALLENGES = 65_536` defensive limit. This sync does not introduce a new challenge-limit decision.
- Take upstream's deliberate boat-math revert (`fdd210261e`, merged by horizon-rust/horizon#15440), retaining fork mod and borrowed-owner adapters. Upstream reports that the reverted change failed its boat-fire parity harness; this session did not run that harness. Keep the corresponding minecart and mount reverts coherent.
- Preserve the richer fork collision-subject context, prediction owner updates and typed fall dispatch while incorporating powder-snow, replay-rebase and fall-sound behavior. Retain custom datapack preset cycling alongside upstream's extended Alt preset selection.
- Retain the fork's repository-owned, paginated release-metadata query and `(pull request number, merge commit)` identity check. Upstream's number-only batching solves a quota problem the fork already avoids, but cannot safely distinguish imported upstream merge subjects. Do not add a second unused batching implementation.
- Resolve locale changes by semantic three-way dictionary merge. There were no divergent same-key values; both sets of additions and intentional deletions survive. No translations were marked reviewed.
- Retain the owner's one-concern, no-size-limit fork contribution policy.

## Diverges from upstream: world hold

The upstream `native_world_hold` / `WorldHold` call is deliberately omitted by maintainer ruling; no replacement raw API is invented. An upstream mod could hold the terrain picture while the world changed underneath, then reveal accumulated terrain changes together on release. That mod-requested terrain-picture freeze/release behavior is not available through this fork's catalog. A typed form with a Host is deferred until a real mod needs it, with any reference counterpart checked through clean-room first. The integration must leave no dead hold code, unused imports or silently ignored hook tests; any removed upstream hook test and the default non-mod behavior audit are recorded in the pull request.

## Move restoration and verification

The integration is prepared in separate commits. The restoration script and the three missing-fix commits ride sync 36; destination moves, reverse window-handler restoration and dead hold-path cleanup ride stacked sync 37, so sync 36 does not wait on twenty destination correction pairs. Three missed post-move fixes are restored in individually named commits: Polytone creative-tab modifiers, streaming upload backpressure, and named profiler spans. The exact compile-only result and independent static verdict are recorded in the pull request when complete. The hold audit confirms no default non-mod effect: the default covers nothing. Existing unused renderer/LOD hold infrastructure predates this sync and is removed in sync 37; no hook, dispatch test or hook documentation is added or silently ignored here. No local tests, clippy, parity harnesses or game runs are authorized for this sync.

## Remaining move metric in sync 36

`measure/game_rs.py` reports **99 name matches**, not zero (initially 104; independent review removed the unused leather-boots duplicate, and the main take removed four more monolithic copies). This approximate, name-based count includes true extraction remnants and unrelated methods with the same name; it is not a qualified Rust-method count. The full destination series is explicitly deferred to stacked sync 37 under the parallel-sync order, while the three missing post-move behavior fixes are already restored separately.

After qualifying the matches, the raw count consists of 90 extraction names, three unrelated `Dim` method names with `game/` matches, and six names matched only outside `game/`. The six names whose upstream matches are entirely outside `game/` are `stage`, `native`, `exists`, `all`, `render_with` and `in_own_process`. `Dim::key`, `Dim::level` and `Dim::sea_level` also collide with unrelated upstream methods. `Options::parse` is handled by the planned whole-Options extraction; its name also occurs in unrelated helpers. None of those facts is used to silently turn the raw count into zero.

All names remaining in the raw metric:

- `a_damaged_shulker_box_spills_its_stacks_upward`, `add_item_entity`, `add_orb`, `add_orb_toward`, `all`, `an_item_escapes_a_block_at_its_own_randoms_speed`, `apply_consume_effect`, `apply_crafter_slot_states`.
- `apply_equipment`, `attack_mob`, `award_furnace_recipes`, `award_orb`, `break_block`, `bucket_hit`, `bucket_target`, `celestial_geometry`.
- `clear_lod`, `click_inventory`, `container_kind`, `crack_particle`, `crack_stage_names`, `cracks`, `creative_display_click`, `creative_throw_carrying`.
- `crumbling_sprite`, `destroy_progress_event`, `drop_bundle_content`, `drop_falling_block`, `drop_inventory`, `exists`, `finish`, `first_person_arm`.
- `hand_geometry`, `held_mut`, `held_slot`, `in_own_process`, `inventory_quick_move`, `joined_during_entity_pass`, `just_spawned_item`, `key`.
- `level`, `load`, `melee_damage`, `melee_damage_with_admission`, `menu_ops`, `menu_session`, `merge_new_orb`, `merge_orb`.
- `mine_block_with_held`, `native`, `orb_merges`, `parse`, `particle_sprite`, `player_break_block`, `player_menu_sound`, `player_name`.
- `pop_resource`, `pop_resource_from_face`, `portal_shape`, `prepare_lod`, `rain_height`, `raining_at`, `random_tick_chunks`, `ready`.
- `release_bow`, `remove_orb`, `render_with`, `run_loading`, `save_level`, `sea_level`, `slot_count`, `slots`.
- `spawn_destroyed_contents`, `spawn_item`, `spawn_item_own`, `spawn_item_thrown`, `spawn_orbs`, `spawn_orbs_toward`, `spilled_motions`, `spilled_stacks_rise_from_their_own_randoms`.
- `stage`, `sync_tick_chunks`, `the_crack_takes_the_size_of_the_set_it_draws`, `throw_from_eyes`, `tick_blocks`, `tick_blocks_with_scratch`, `tick_items`, `tick_moving_pistons`.
- `tick_scheduled`, `tick_weather_effects`, `transform_particle`, `try_light_portal`, `unstuck_orb`, `update_world`, `used_hand`, `wears_freeze_immune`.
- `weather_geometry`, `with_hand`, `write_report`.
