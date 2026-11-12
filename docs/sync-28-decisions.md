# Sync 28 integration decisions

Target: horizon-rust/horizon at `d9b9996a4dfc61d0316daf599d7f937ccb8b95a8`.
Fork base: `5724caf147`. Common ancestor: `501446443c`.
Upstream changed 659 paths; the initial merge produced 204 conflicted paths.

## CI ownership

Keep all fork-base `.github/workflows/` and `tools/ci/` files. Upstream's runner and parity policy changes do not replace this fork's Dyna checks, static verification, runtime leases or release configuration. Retain the fork-specific parts of CI runner documentation. No CI dispatch is performed while the rollup gate holds runners.

## Launcher uninstall and identity

Take upstream's moved-data uninstall handling and its build-directory ownership guard. The default data-directory ownership check continues to use the fork's complete identity namespace rather than upstream's generic `horizon-launcher` leaf. The incoming default-path test uses the configured launcher identity so it applies to both builds. Moved locations keep upstream's `launcher`, `game` and `build` layout, and never delete the game directory.

## Developer tools and documentation

Combine upstream's optional `--harnesses` self-check flag with the fork's exact revision and dirty-worktree reporting. Preserve fork executable names in the controller harness instructions. Take upstream's new on-screen keyboard cases without changing their assertions. Layer documentation takes upstream's page form API but retains the fork dependency inventory rather than replacing it with upstream's exception count. Layer allowances preserve fork dependencies alongside incoming telemetry classification.

## Port ledger and translations

Merge port-ledger rows by Java method key: unchanged upstream rows retain the fork's relocated paths; changed upstream contracts replace the matching row, with unambiguous source paths relocated to the fork's game-area layout. Merge Thai launcher translations by key, taking upstream changes while retaining fork-only keys. Gameplay and rendering translations are resolved in their owning areas.

## Engine, item and server integration

Audited all 22 assigned paths and the additional random.rs correction. Two relocated destinations are also changed: crates/horizon-server/src/lib.rs and crates/horizon-server-launch/src/config.rs. Total: 25 paths audited, 23 changed, 2 deliberately unchanged. No unresolved textual cases or requested rulings.

- Item model tests retain both the registered mod node test and upstream linear/binary threshold coverage.
- The three language files retain fork keys and take upstream added/changed translations, including Horizon UI text.
- NBT: upstream changed its old horizon-world/src/nbt.rs, now relocated to horizon-core. Ported Cursor/Key and entry/list header primitives there, preserving fork heap quotas, bounded reads, strict duplicate handling, split recursive frames, and list_fits validation. Cursor and Key must be public because structure template decoding is now in a separate crate. No obsolete world NBT file restored.
- Items retain explicit DAMAGE presence and include_damage gating. Upstream prototype-aware removal of equal DAMAGE 0 is integrated at serialization. Upstream custom durability/mining modules, tag replacement, weapon blocking, and consume parsing integrate with existing fork modules. The duplicate Items attribute_modifiers implementation is not introduced: the fork's ItemStack::default_attribute_modifiers already owns that behavior and incoming tests already call it.
- Protocol registry.rs remains unchanged from the fork base: fork registry count limits and registry-scoped tag replacement retained; upstream conflicting tag changes are documentation/parameter naming, not a missing behavior.
- Server files expose both durable and incoming chat_guard.
- Server backup security, pruning, rollback on failed restore placement, and upstream tests integrated. Interval UTF-8 and overflow corrections and tests ported to the relocated Backups::interval in server-launch/config.rs, rather than restoring duplicate backup::interval.
- Server engine retains server-launch command construction; upstream parallel_tick is forwarded by environment without duplicating launcher logic. Control line-break rejection and its incoming regression test integrated. Parallel execution remains disabled at the single fork gate.
- Server main.rs remains the thin library wrapper. Incoming signals::stopped cleanup is applied in relocated server/lib.rs instead. Supervisor retains launch helpers and takes upstream queued restore/update handling before every start.
- Piston events keep note_override and take upstream queue/pop deduplication routing. Existing upstream level redstone and brightness tests included.
- Structure changes take upstream streaming template decoding and saved legacy piece reads/tests. Imports use relocated horizon-core NBT/BoundingBox and retain fork block entity transform module. Saved-template int/flag use the new shared saved_legacy helpers rather than duplicate implementations.
- LegacyRandom state/gaussian conversion initializes gate to None, matching independent clone/from_state behavior. No Mob RNG API types changed here.

## Clean-merge correction and module placement

The native explosion state merge saved blast ownership twice, then restored empty duplicate values over the outer blast attribution. Remove the duplicate saves and restores, retaining the incoming player-owner flag and the fork nested-blast contract. New extinguish, frame autotuning and smithing icon modules move into their existing entities, runtime and interface areas through the committed relocation script.

## Parallel mob dispatch: authorized performance divergence

Diverges from upstream (performance only). The fork retains serial dispatch while preserving upstream parallel implementation behind one fork-owned gate, with minimal typed adapters where required. Upstream horizon-rust/horizon#15493 states “Not yet shown to give a CPU gain.” The originating horizon-rust/horizon#15056 states “This draft does not yet establish a worthwhile performance gain or acceptable fallback overhead.” No upstream measured gain is claimed here. Tests and serial behavior remain unverified locally.

## Contracts and structural rulings

- Preserve JoinList and its validated mutation boundaries, typed hook contexts, fork Parts vocabulary and Wire API. Do not replace them with upstream MobSlot slices or contextless services.
- Preserve Mob.random, Mob::with_random, mobs::entity_random and living_random as LegacyRandom. New PlayerMotion.random is EntityRandom, constructed with EntityRandom::own(mobs::entity_random(...)) in the existing player_init destination. World.random stays LevelRandom; no world RNG reversion.
- Projectile owns exactly one random stream in Projectile.random: OwnRandom(LegacyRandom). Constructor consumes UUID's first two draws from that stream; shoot_own advances the same stream used by arrow ticking. ArrowState::new() stays unchanged; do not introduce ArrowState.random. shoot_from_rotation uses upstream's no-RNG-argument signature. with_entity_random still overrides the owned stream for fixtures.
- Preserve typed blocking damage and shield_damage. Add blocking_item_used through Environment, PausedHurt and BlockOutcome, initialize false, copy across suspended equipment wear and set when blocking processing runs. The removed health/equipment.rs is not restored.
- Upstream external renderer seam, linear-light constants, dimension-aware rendering, chat guard interface, far-ring fairness, banner tag support, item consumables and NBT narrowing are integrated with fork APIs rather than importing obsolete source layouts.
- Integrate incoming behavior into the active fork owners instead of keeping undeclared upstream extraction modules. Air-supply and walk-target fixes are explicitly ported from upstream into the typed AI pipeline; the other extractions retain equivalent relocated or inline implementations: renderer census/frustum helpers, player_init, options layout/presets with native controls, mob seen/pending pushes, entity NBT armor stands/rockets/orbs. Incoming behavior is integrated at the live implementation. PendingPushes additionally gets Clone and append for upstream helper code.
- Preserve the fork asynchronous screenshot writer and its typed hooks rather than replace it with upstream's different write-completion API. Runtime window_handler adopts frame_autotune for dynamic resolution while retaining typed pause hooks. Existing trace/span logging remains there.
- world_warp.rs and world_warp_fragment.wgsl remain deleted; their renderer-side warp replacement machinery was already removed by the typed render architecture. No old warp test or mod service is restored. Inter-stage coverage takes the new ray-unpack test only.

## Path audit

### Renderer

- Cargo.toml: retain mod dependencies; take removal of Horizon cosmetics dependency from the engine layer.
- block_entities.rs: merge sign editor tracking with attachment generations and observers.
- gfx.rs: external pipeline/frame policy/linear light integration; keep prepared model and render hooks; shared entity factory accepts linear-light mode; preserve primitive reserve and census helpers.
- gfx/inter_stage_tests.rs: new ray-unpack coverage, no deleted world warp tests.
- gfx/world_primitives.rs: linear frame shader contract plus fork stage/capacity contract.
- gfx/world_warp.rs and gfx/world_warp_fragment.wgsl: keep deleted, as above.
- hud.rs and hud/pack_text.rs: combine model-reducing and menu picture modules; split GUI/world text shaders while retaining ResourceSource.
- options_ui.rs: take chat-delay and pack-drop behavior and widget visibility; retain native settings and existing extracted layout; preserve native slider events and use upstream four-stop preset mapping.
- packs.rs: copy_pack export and case-insensitive zip detection; retain resolved source contract.
- particles.rs: camera quaternion rendering plus retained starter/ordinary pending queue and resource source test; add quad rotation test using indexed source.
- particles/polytone.rs: upstream particle behavior with fork pending enum lookup.

### Client systems

- banner.rs: merge pattern tags with fork ItemDefaults loading; keep item metadata ownership.
- client_lib.rs: expose parallel and dynamic_resolution, retaining block_sounds and fork mod/render reexports.
- entities.rs: obstacle/water-splash behavior and entity data changes, preserving context-bearing fluid updates and before-velocity splash input.
- entity_nbt.rs: upstream byte/short codec behavior, horse variants and regression tests; preserve full typed equipment and attachment/join initialization. Keep inline fork armor stand/orb/rocket implementations rather than duplicate imports. Use tame restore_sitting.
- entity_nbt/hanging.rs: loaded pitch and byte-wrapped facing; retained item-frame implementation in parent uses those helpers.
- entity_nbt/projectile_fields.rs: projectile-kind codec behavior with fork horizon_core NBT import.
- entry.rs: install upstream chat filter; retain fork horizon_look path.
- game.rs: baseline dimension type/environment/render changes and tick spans; preserve typed harvest/critical/sweep/orb hooks. Includes relocated gameplay behavior without restoring deleted root modules.
- health.rs and hunger.rs: blocking_item_used propagation, typed damage retained.
- health/equipment.rs: stays deleted; its field propagation is in active health.rs.
- item_sync.rs: consumable codecs and imports, retaining fork attribute/kinetic modules and NBT layer.
- item_tooltip.rs: upstream item display behavior with typed attribute search context retained.
- lan/hub.rs: upstream chat guard interface, air-state table checks, duplicate teleport refusal, teleport-id invalidation, far-ring scan cursor and missing-column retries; preserve JoinList updates, watch notifications, snapshots and typed effect particles. sync_data accessor added.
- lan/hub/chat_filter.rs: import server-files guard contract and Place; no direct engine-owned Horizon filter guard.
- lan/hub_tests.rs: stays deleted. New cave-air/far tests are wired into active inline hub tests; duplicate-accept regression moved there.
- lan/io_pool.rs: memory-connection cap exemption; obsolete fork-removed packet-tally tests not restored.
- menu.rs and menu/place.rs: quick-move take helper and placement behavior combined with fork typed inventory inputs.
- mobs.rs: retain JoinList/damage/target contracts and rich StoredEffect values; port client-side damage guard, visibility/foreign-seen data, random ownership boundary, tame pose and shared model extraction. Keep fork inline seen/pushes implementations; append/Clone support added.
- mobs/ai.rs: camel age, fire reset/powder-snow behavior with existing typed air/damage/consumption pipeline; do not restore duplicate extracted walk-target/contact tests or contextless attack routine.
- mobs/aquatic.rs: unchanged after audit; upstream delta only introduces generic MobSlot access, refused in favor of JoinList validation.
- mobs/conversion.rs: preserve full typed StoredEffect/absorption conversion, including icon copying; take unrelated upstream deltas.
- mobs/enderman.rs: upstream behavior around existing typed target approval rather than direct target assignment.
- mobs/equine.rs: horse variant codecs and filtered bolt destination goal; retain typed test level.
- mobs/living.rs: drowned current immunity and boat fluid interaction state; preserve active fork movement hooks.
- mobs/piglin.rs: incoming behavior with typed target proposals and concrete player identity retained.
- mobs/riders.rs: boat/fluid behavior; preserve JoinList navigation loan and paused-consumption restoration rather than MobSlot ticking.
- mobs/spawning.rs: runtime finalize regressions adapted to StoredEffect values and typed helper.
- mobs/spear_use.rs: unchanged; upstream's event change is contextless player-event routing that the fork already replaces with typed routing.
- mobs/turtle.rs: unchanged after audit; retain typed TestLevel and LegacyRandom rather than incoming generic RNG test rewrite.
- mobs/wet_fire_tests.rs and mobs/zombie_villager.rs: upstream behavior/tests adapted to retained Mob LegacyRandom and profession data.
- motion.rs: retain player_init instead of duplicate body module; add entity-owned player random through existing initializer.
- multiplayer.rs and title.rs: upstream title entry/field handling and world-folder naming with fork screen transitions retained.
- projectiles.rs: unified RNG/UUID contract described above; rotation/fire/below-world changes retained; no contextless block-contact service added.
- save.rs: upstream save backpressure plus fork ticket serialization/fluid interning.
- vanillagen/structure_mobs.rs: background load owns Arc<ItemData> and uses its defaults; retain contextual fork cache initialization.

### Entities, network and world

| Assigned path | Resolution |
| --- | --- |
| `crates/horizon-client/src/game/entities/armor_stands.rs` | Upstream special-hit admission/tail and dimension-type lighting; victim identity remains typed. |
| `crates/horizon-client/src/game/entities/boats/below_world.rs` | Created at assigned relocated destination; validated JoinList removals and below-world parity regressions. |
| `crates/horizon-client/src/game/entities/boats/mod.rs` | Float-exact paddle/escape vectors, freeze counter, last-Y movement and below-world behavior; typed held-out mutation retained. |
| `crates/horizon-client/src/game/entities/entity.rs` | Kept fork loop-context block-effects implementation instead of upstream hookless ItemTables move/settle extraction; added item air fixture field. |
| `crates/horizon-client/src/game/entities/explosion.rs` | Explicit explosion source/credit behavior merged through DamageLevel; upstream regression uses typed LevelRef explosion boundary. |
| `crates/horizon-client/src/game/entities/frames.rs` | Projectile frame hits, creative-owner drops and dimension lighting; typed hook rotation remains normalized. |
| `crates/horizon-client/src/game/entities/minecarts.rs` | Freeze state, below-world removal, exact control yaw; moved typed attack into special_attacks; kept live ownership and NBT freeze persistence. |
| `crates/horizon-client/src/game/entities/pvp.rs` | Takes upstream PvP behavior in the existing entities area, not a duplicate extras module. |
| `crates/horizon-client/src/game/creatures/mob_group.rs` | Byte-identical upstream source retained at original upstream path behind nondefault source-retention feature; not compiled. |
| `crates/horizon-client/src/game/creatures/mob_islands.rs` | Byte-identical upstream source retained at original upstream path behind nondefault source-retention feature; not compiled. |
| `crates/horizon-client/src/game/mobs/creatures/mod.rs` | Kept JoinList lift/restore + BorrowedMobTick, ported observable event/ignition/projectile/UUID behavior and caller RNG signature. Source-retention declarations isolate incompatible parallel extraction. |
| `crates/horizon-client/src/game/creatures/prep.rs` | Byte-identical upstream source retained at original upstream path behind nondefault source-retention feature; not compiled. |
| `crates/horizon-client/src/game/mobs/creatures/projectile_targets.rs` | Created relocated active pickable/projectile-target implementation, including primed TNT and item frames; retained separate fork vehicle targets for typed callers. |
| `crates/horizon-client/src/game/mobs/creatures/projectile_world.rs` | Kept fork context-owned block-contact path; ported projectile world floor query. |
| `crates/horizon-client/src/game/mobs/dragon.rs` | Cloud velocity and target computation updated; moved typed special attack to items/special_attacks. |
| `crates/horizon-client/src/game/mobs/enderman.rs` | Placement survival and non-mob occupancy integrated around existing typed leave-block decision/apply transaction. |
| `crates/horizon-client/src/game/mobs/hoglin_repeat_attack_tests.rs` | Retained typed target proposal contract and appended two upstream broadcast distance regressions with source-player connection adaptation. |
| `crates/horizon-client/src/game/mobs/mounts.rs` | Upstream public escape vector; Mob RNG remains LegacyRandom by client ruling. |
| `crates/horizon-client/src/game/mobs/ominous_item_spawner/mod.rs` | Loaded rotation normalization uses fork core NBT types and mutable owned input. |
| `crates/horizon-client/src/game/mobs/raids/mod.rs` | World owner RNG swaps wrap LegacyRandom in LevelRandom; scoped patrol draws preserve stream. |
| `crates/horizon-client/src/game/mobs/spawner.rs` | World owner RNG swaps use LevelRandom::for_level; no extra draw. |
| `crates/horizon-client/src/game/network/lan/mob_states.rs` | Active relocated serial fill_mob_state adapter reads typed live attributes. Upstream parallel Later/AttrSources requires Sync Mob and is not used. |
| `crates/horizon-client/src/game/network/lan/mod.rs` | Metadata uses upstream sitting pose while preserving typed effect-particle modifications and live attributes; serial state helper extracted; broken stat added. |
| `crates/horizon-client/src/game/network/lan_actions/tests/guest_body.rs` | Upstream extraction relocated to production lan_actions/guest_body.rs, not obsolete tests location. Constructor failure still refuses turn before state loan; writes through Guest JoinList::update. |
| `crates/horizon-client/src/game/network/lan_actions/tests/guest_body/tests.rs` | Upstream saved-body/distinct-stream regression retained at production helper test destination. |
| `crates/horizon-client/src/game/network/lan_attributes.rs` | Fork live-map metadata kept as source of truth; upstream equipment-derived parallel AttrSources would duplicate components and does not compile against !Sync Mob. |
| `crates/horizon-client/src/game/network/lan_mobs.rs` | Preserved fork typed hoglin target transaction; ported Ignite/ZombieIgnite routing. |
| `crates/horizon-client/src/game/network/lan_vehicles.rs` | Preserved Guest JoinList::update validation and typed host-borrow contract; no obsolete mutable guest indexing. |
| `crates/horizon-client/src/game/network/remote/mod.rs` | Kept typed tag notices/residency; ported item-use/abilities/gameplay tag extraction, source sound and respawn attributes; removed duplicate UpdateTags dispatch. |
| `crates/horizon-client/src/game/network/remote_mob_data.rs` | Sitting pose metadata integrated. Existing typed villager receive already owns Enderman stare sound, so no second scratch-state implementation. |
| `crates/horizon-client/src/game/network/remote_vehicles.rs` | Client-side horse damage marker integrated with fork LentLevel tick. |
| `crates/horizon-client/src/game/world/end.rs` | Gateway delay phases use existing typed dimension_changing_delay instead of removed contextless entity helper; dimension-type environment/lighting kept. |
| `crates/horizon-client/src/game/world/integrated.rs` | Catch/save integrated tick failure merged while retaining crashed flag and typed close lifecycle. New regression uses fork test factory. |
| `crates/horizon-client/src/game/world/levels.rs` | Upstream per-level spans retained around typed enter_level and hook LevelRef bodies, draining pending inside-block events before post. |
| `crates/horizon-client/src/game/world/packs/mod.rs` | Upstream drop/import UI integrated with mod-discovered additional packs and caller update. |
| `crates/horizon-client/src/game/world/persist.rs` | Retained fork transferred-object initialization, passenger admission and typed object-tree persistence. Ported crystal fire cleanup, no-save leash knots, portal cooldown saves and camel reload regression; destination object_memory gains own projectile UUID and boat freeze state. |

## Destinations outside original list

- crates/horizon-client/src/motion/player_init.rs: initialize new entity-owned player RNG without duplicating initializer.
- crates/horizon-client/src/game/runtime/window_handler.rs: upstream frame_autotune invocation at fork relocated frame completion point.

## Integration decisions

- Deleted root gameplay paths stay deleted. Their changes land in game.rs, blocks, items, mobs, network, runtime, world, entities and modding destinations, not restored obsolete modules.
- Retain JoinList update closures, residency extraction, validated guest turns, scoped invulnerability checks, typed damage/attack/victim identity, typed spawning, Parts layout and Wire contracts.
- Incoming projectile own-RNG shooting retains spawn_projectile rather than restoring add_projectile. Incoming parallel item moves are wired through game.rs's ahead module, with per-turn saved invulnerability ticking retained and the existing native-hook refusal in ahead preserved.
- Random tick chunks are sorted deterministically, with masks prepared before ticking; tests are explicitly wired from game/block_ticks/random_order_tests.rs.
- Item void bounds use the live level minimum. Existing fork escape tests retain independent seeded expected values and ticking-neighborhood preparation rather than replacing them with implementation-derived expectations.
- Guest body initialization remains inline in the validated JoinList turn, not upstream's unchecked mutable-Guest extraction. Its own entity RNG is initialized before saved data is applied. Drop resynchronization runs through self.as_guest and updates the guest's told-selected state afterward.
- Merchant refresh signaling and OffersRefreshed detection retain fork-owned LAN borrowing signatures. Reputation changes refresh prices; ordinary takes no longer resend offers unnecessarily.
- Keep fork damage_stack, harvest/break-speed hook, tooltip hooks and native ram combat adaptations instead of restoring upstream's obsolete implementations. The existing fork helper already covers mob_pushes_player with explicit attack context.
- melee_damage_with_admission keeps victim: Option<u32> as a third argument, alongside the old typed melee_damage wrapper, so damage RNG is evaluated once while exposing admission.
- Preserve ItemConsumable's synchronized prototype/data/wire lookup through Game::consumable while keeping typed can_eat and hook calls. Borrow the returned owned consumable for completion.
- Keep sync 27's refusal to restore entity_portal_delay and contextless native RNG/entity services. Take live portal cooldown serialization; dimension scaling now uses DimensionFacts.
- Keep the existing waypoint Icon vocabulary; persistence is implemented around it, without introducing a duplicate WaypointIcon type.
- Take upstream Horizon cosmetics settings, PvP helpers, dedicated crash-stop behavior, beacon text, sign edit lifecycle, crack sprites and world-update spans. Existing fork save-before-unload hooks remain in their established order.
- New position_args validation lands at administration/commands/position_args.rs, cosmetics settings at interface/cosmetics/settings.rs, and renamed upstream extras/pvp.rs merges into entities/pvp.rs. No duplicate extras/pvp.rs is restored.
- World content access uses the existing Game.server_items cache before local resource items. Enchantment readers use Remote.enchantments. The upstream ServerTables aggregate is not duplicated: synced tag/item/enchantment updates already populate these flat fields, and typed mod tag notices remain separate.

## Diverges from upstream (performance only)

Single fork gate: `crates/horizon-client/src/parallel.rs::parallel_tick`, local constant `FORK_PARALLEL_TICK_ENABLED = false`. It disables ALL opt-in parallel server phases, not only mob phases. The upstream pool, par_map and verification machinery otherwise remain in the tree.

Exact type blockers: `mobs::Mob.extra.live` is `living_attributes::LivingAttributes`, whose map is `std::cell::OnceCell<Box<Live>>`. `Live = LiveAttributes<Box<dyn Identities>>`; `Identities: FnMut() -> i32` lacks Send and Sync bounds. `LiveAttributes::data` owns that allocator callback. OnceCell -> OnceLock alone therefore cannot make Mob Send + Sync; no unsafe shim or cache refactor was taken. The fork tick additionally owns `JoinList<Mob>` and `health::LentLevel` (LegacyRandom, attachments and syncs), not upstream plain Vec tick ownership. Smallest active adapters remain `BorrowedMobTick` and extracted serial `fill_mob_state`, reading the existing authoritative live attribute map.

Upstream `game/creatures/{mob_group,mob_islands,prep}.rs` are retained BYTE-IDENTICAL at upstream paths. Only their module declarations are behind nondefault Cargo feature `upstream-parallel-mobs`; no default enables it. **This code is not compiled or tested on the fork.** Do not turn that source-retention feature on to claim working parallel dispatch. No currently compiled fork code calls those modules. Their eventual typed adaptation is fork #3491.

## RNG adapters

World swaps use `LevelRandom::for_level(LegacyRandom::new(...))`. Hook-owned LentLevel continues to expose LegacyRandom via the world owner's deref adapter, coordinated by parent/client. Mob.random and mobs::entity_random remain LegacyRandom by client ruling. Projectile shoot_from_rotation uses upstream no-caller-RNG signature; own_uuid/uuid are upstream projectile contracts expected from engine/client integration. No RNG draws added by wrappers.

## Upstream behavioral tests retained and statically assessed

- `a_vehicle_falls_out_of_the_world_64_blocks_under_the_floor`, `a_tick_discards_the_vehicles_under_the_world`, `saved_deep_end_boat_loads_then_is_discarded_and_extreme_scans_terminate`: serial removal happens before motion, spills container contents without ENTITY_DIE; no cross-thread behavior required.
- `a_carried_block_is_not_placed_into_an_entity`: placement validates survival and non-mob occupancy inside the existing typed decision/apply path.
- `a_broadcast_player_is_compared_by_distance`, `a_guest_broadcast_is_placed_as_the_guest`: typed target proposal compares source/current player identities and commits after hook approval.
- `co_located_guests_own_distinct_streams_and_restore_the_saved_body`: validated Guest update initializes the body with per-ID stream before saved state restoration.
- `a_cat_on_a_chest_sits_for_the_other_players_too`: metadata uses sitting_pose, not order state.
- `flint_and_steel_use_on_ends_the_click`: fire prediction sits after fork hook, cooldown and adventure gates and terminates the click on PASS-excluding admission.
- `a_servers_block_and_item_tags_replace_the_games`: gameplay items refresh independently of mod presence; typed mod notices remain distinct.
- `gateway_counters_match_actual_vanilla_methods`: real game identity-based delay selection replaces removed Entity trait helper; arrow/pearl phase ordering retained.
- `integrated_tick_panic_saves_changed_world`: catch-and-save loop preserves shutdown error and typed session teardown.
- `explicit_sources_keep_names_without_inventing_kill_credit`: preserved through typed LevelRef explosion call, no contextless service added.
- Frame projectile regression module and existing camel reload assertions retained. Parallel-only structural tests inside the retained upstream sources are not compiled/tested, as stated above.

## Rollup 46 main integration

Merge current main `b96005b7c9115c9061fa996e00b5b029a7fc0efc` before publishing. The two inventory conflicts keep main's Menu/State/touch structure while preserving upstream quick-move result accounting and viewer-aware recipe transfers. Craft results report the crafted copy minus leftover; merchant results report the moved stack. Transfer snapshots and drop truncation read State, and mutations call touch. Test callers pass the existing viewer argument explicitly.

## Native hook declarations

The incoming native_hooks.rs change dereferences the new level RNG wrapper inside an obsolete contextless service. The fork removed that service; no hook declaration or capability is added back. Existing typed host and RNG contracts remain. Wire, derive, hook catalog and the parts vocabulary layout are unchanged by this sync.

## Compile integration corrections

Incoming structure, item and entity code uses the fork's relocated `horizon_core::nbt` boundary, including its SNBT parser. New item-table fixtures load installation-scoped ItemDefaults explicitly. The accidentally restored lever veto test was already removed on the fork base with its contextless redstone callback; that obsolete test is not restored, and no replacement capability is claimed.

The forced-ring ranking helper reads the single authoritative combined `Streaming::forced` set, which includes command and mod tickets, rather than restoring the removed separate mod-forced set. Ranking and queue invalidation use that same combined set. Incoming forced-ring assertions are retained.

Source discovery explicitly excludes the retained upstream `game/creatures` folder; feature-gated declarations own those source files. The extinguish regression is nested under its owning module and uses scoped resource fixtures, preventing duplicate module discovery and resource leaks. Duplicate renderer and remote-item methods from upstream extractions are removed while retaining text-patch invalidation and typed interaction gates.

## Final upstream call-site integration

- Resource-pack drops use `drop_pack_file` from the existing window event path, retaining Create New World handling and adding upstream's resource-pack-screen handling.
- Portal coordinate scaling uses `DimensionFacts` from the active game dimension context; the orphaned static Dim helper is removed instead of bypassing synced or mod dimension facts.
- `/execute in` retains the existing validated `Dim::exists` / `Dim::from_id` path, including mod dimensions; the redundant `known_level` helper is not retained.
- Login/respawn uses `drop_remote_player` to capture carried attributes, then performs the fork's `remove_local_player_body` teardown so hooks, residency and attachments remain correctly invalidated.
- Remote spear attacks synchronize look and report in-range attacks before action 8, retaining swing and sound behavior.
- Guest PvP mace hits reuse `pvp_hit_after` and send updated motion immediately before Wind Burst. Existing exhaustion, weapon-hit and sprint-stop behavior remains.
- PvP glint and lower-fire rendering use their existing helpers rather than duplicate inline implementations.
- Biome resend batches identical packet bodies across watching guests through the existing validated guest read boundary. The outgoing per-receiver column set and ordering are unchanged.
- Projectile RNG remains a single owned stream; no duplicate ArrowState stream is restored. UUID allocation and shooting consume that stream, and native launch/rotation fixtures inspect the same stream.
- New regression fixtures use scoped resource ownership, validated GuestList mutations, typed entity slots and installation-scoped item defaults. Orb air and rocket fall distance serialize through the retained inline typed entity codecs. No assertion was intentionally weakened.

## Verification scope

Only compile-time workspace checks and scoped Rust formatting run locally for this sync. Tests, clippy, parity harnesses and game behavior are not run locally. The retained upstream parallel sources are excluded from compilation and testing. PR readiness uses verify, verify-static, lint, launcher and Windows test; wasm fixture and fn-replace-check are main-only, not PR gates.

## Review carry: duplicate source and regression coverage

Nine undeclared upstream extraction files are removed after checking explicit module declarations, path attributes and discovery ownership: options_ui_preset; lan/hub/accept_teleport and chunk_batch; mobs/ai/air and walk_target; mobs/mob_slot, pending_pushes and seen; motion/body. The active options layout, inline validated hub code, inline mob state and motion/player_init retain the equivalent upstream behavior. The active typed mob AI pipeline now ports the upstream air and walk-target changes; those two extraction files were bug fixes, not merely duplicate code. The three explicitly retained parallel mob sources are unaffected.

The upstream first-tick armor-stand movement-speed persistence correction is present in the inline serializer, before fork live-attribute packing. Saved attribute order is preserved.

Twelve of the thirteen requested upstream regressions are carried: shield use statistics, client-copy fall immunity, lectern removal events, four waypoint icon persistence cases, armor-stand pitch, wrapped hanging facing, cloned held attack items, merchant packet snapshots and resource-pack drops. Test bodies are copied from the pinned upstream head with typed names and scoped resource fixtures adapted. Their upstream resource-dependent ignore annotations remain.

Not carried: `parallel_tick_reaches_the_game_only_when_it_is_on`. Upstream's test reads the environment from its command-construction seam, which accepts a parallel flag. The fork's `horizon_server_launch::command` has no parallel flag; `engine::spawn` adds the environment and immediately spawns the process. Adapting the test would require a new production seam or subprocess fixture rather than typed-name substitutions. Environment forwarding remains present. The two incompatible parallel-mob state-path tests remain intentionally uncompiled as already recorded.

## Delta carry: air, stroll scoring and extraction tests

The active AI air branch ports upstream's tag-plus-overrides breathing rule (baby happy ghast and block-body sulfur cube), and iron golems consume zero air rather than being incorrectly added to the breathing tag. Walk-target scoring ports zero scores for endermen, pillagers and wardens, plus the mooshroom mycelium preference. The upstream air and ground-score regressions are restored, as are graphics preset floor-stop and ordered appended-push tests. These behavioral changes are ports, not existing duplicate implementations.

The other seven deletions are rechecked against their active owners: graphics slider/preset helpers are equivalent with additional native-setting events; validated hub teleport acceptance retains duplicate acceptance refusal and recording reset; chunk batching retains light-neighbor gating, quotas, bounded far scans and cursor updates. PendingPushes and Mob::seen retain the inline typed implementations; player_init retains identity allocation and wraps the same random stream in EntityRandom. MobSlot remains deliberately refused in favor of JoinList validation. No alternate source modules or compatibility paths are restored.

## Current-main integration after delta carry

Merge main through `66a48904198` once after the C1–C4 carry. The sole textual conflict is the post-removal harvest check in game.rs: retain main's `player_destroy_block_removal` result, whose `harvest_after_removal` already evaluates the current held tool and typed mod harvest check. Do not run that check a second time. All other main changes merge directly, including its CI configuration and rendering/mod architecture updates.

## CI repair: lint and static tool contracts

Read current-head lint and verify-static logs before repair. Remove the redundant i8 item-frame rotation cast, apply current rustfmt to remote projectile dispatch, and remove the repeated telemetry edge from the layer allowlist. The entity-lift source check excludes exactly `game/creatures/mob_islands.rs`: it is byte-identical retained upstream source behind the unsupported nondefault feature, excluded from active module discovery. Its raw take is not changed, and no active entity-list mutation is exempted. Python tool tests follow the actual retained tool contracts rather than obsolete fixture argument shapes. No local tests or clippy run for the repair; CI is the check.

## Frozen-main payload and platform integration

Merge frozen main `a51728c35f8` once, retaining #3087 raw custom play-payload transport, #3038's registrar/tables vocabulary placement under mod-api/src/parts, and #3504's engine-owned platform socket filled by horizon-mod-host. The four conflicts are narrow: retain both LAN module declarations and both hub test groups; decode inbound play through the negotiated payload codecs; keep upstream's single frame-header parse while delegating that parse to main's memory-aware `header` implementation. No raw payload or socket implementation is removed.

## CI verify and remaining gameplay lint repairs

Read verify's five named failures and lint's three gameplay errors on `c14c002fb04`. Correct item-frame rotation decoding to narrow to signed byte before remainder; refuse client-copy mob damage before typed invulnerability observation; pass furnace output's remaining stack to the subsequent onTake notice without changing crafting result behavior. Align the orb fixture's Air with the existing 32767 assertion and initialize saved consumable components through StackData's typed setter. Assertions and expected values remain unchanged. Remove the orphan minecart doc line, collapse the cart attack condition without changing short-circuit order, and move the dedicated crash-stop implementation before its test module.

## Polytone admission and retained-test sweep

The pending-particle enum adaptation accidentally kept fork construction-time counting beside upstream's newer admission tests: physics candidates that might never join consumed a type limit, pending candidates incorrectly participated in exclusion-radius checks, and the ordinary-particle admission branch omitted upstream's per-type limit recheck. Restore upstream `polytone_joins` and call it from ordinary admission. At construction, only pending particles without spawn checks count toward limits; exclusion radius considers only live particles. Existing test expectations are unchanged.

The static pattern sweep compares conflict-file upstream test inventories and changed implementations across particles, other rendering, gameplay/client-library, and engine/item/world/protocol/server areas. It preserves deliberate typed API, resource-source and mod-layout divergences rather than replacing them wholesale. A successful all-target compile does not establish test success. The failed verify stopped at the rendering library; later workspace binaries, ignored content-root tests and docs remain for CI.

By root order, the one permitted CI change in this repair is adding `--no-fail-fast` to the existing runtime, documentation, launcher and Windows Cargo test commands, with their two pinned command strings updated. The edit is byte-identical to the corresponding commands in #3509; its separate compile-only timing step is not carried. No other workflow lines change.

The conversion sweep found one deliberate divergence from upstream: retain vanilla-correct absorption conversion, clamping copied absorption to the target maximum before restarting copied effects (fork commits `e4921353d7` and `e5c7b781a5`). Vanilla 26.3 `ConversionType.convertCommon` calls the target setter before copying effects; `LivingEntity.setAbsorptionAmount` clamps to the target maximum. The incoming preservation fixture now grants the target maximum absorption 3.0, retaining its original preservation assertion and existing zero-clamp/effect-restart regressions. No conversion production behavior changes.

## Sync 27 repair

The Allay healing regression still used the former `extra.ints[3]` pickup-cooldown storage. Its fixture now uses `set_pickup_cooldown`/`pickup_cooldown`, retaining healing, age, duplication, note-block cooldown and NoAI assertions. The typed optional memory deliberately distinguishes absent from present zero. Gameplay is unchanged.

The villager witness regression is an oracle-harness step, not a standalone slow-shard test: it requires `HORIZON_REPUTATION_ORACLE`. Register it with the existing `harness` convention using the verified driver `tools/test_villager_witness_parity.py`, which generates and supplies the oracle. The Rust test still requires that oracle; no skip-if-missing behavior is added.

The HDR near-black dither failure remains unresolved statically. Its test, shader and active GPU gating match upstream; no fork behavior difference or justified software-renderer exclusion was found. Preserve tolerance, shader and gating, and add the measured mean, expected range, target PQ code and input grey to the assertion message so a recurrence supplies the missing evidence. This is diagnostic-only, not a claimed HDR fix.
