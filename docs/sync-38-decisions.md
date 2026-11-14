# Sync 38 decisions

Base: 1a1b026ac73b086dcae413df50e5be8cc31d0e8b. Upstream: d1b6cd286ea4ebafd4e454829e199a867937b77a, own checks run 43494998063 succeeded. This is beyond the earlier 298c01b0206 pin.


## Bulk stamp

Upstream horizon-rust/horizon#16107 is predominantly replacement of API fragment placeholders with ordinal API numbers. It touches old file locations because the fork moved these implementations or replaced the old route. Modify/delete conflicts retain fork deletions; canonical matching basenames are recorded in decisions.md where present. The fork's native_hooks.rs monolith is replaced by individual typed hook modules under game/native_hooks/; native-hooks.json is replaced by generated typed API documentation/catalog machinery. src/mod_api.rs was deliberately deleted (9d6e79400b1, Remove orphaned former mod API module). None of these files should return merely to carry an ordinal comment.

The seven extant implementation conflicts plus docs/native-mods.md retain fork files: bots preserve PlayerRef and typed collision queries; mobs preserve the actual in-place spawn contract; panel does not restore NativePanelStyle; player break keeps crate visibility; structures keeps typed snapshot mapping/refusal; client hooks keep the typed dispatch; native world columns keeps its typed Heightmap and ColumnBlocks paths. Native docs retain the book pointer and current fork contracts rather than resurrecting the old native ABI manual. Incoming differences in these files apart from terrain surface are stamp-only.

## New terrain capability: integration still required

horizon-rust/horizon#15741 (merge 25ad58898ec) genuinely adds a generator query, not a duplicate loaded-height query. Fork surface_heights/client_surface_heights/read_columns read loaded blocks; they cannot answer an unloaded, undecorated generator column. The new upstream native_terrain_surface returns, for each x/z column, an optional value containing ocean-floor height, surface block identifier, optional fluid-top height, and biome identifier. It asks the dimension's own generator through end.locator(), worker(), and lod_surface(); biome names come from world_biomes!(self), not the base biome table. It creates no chunks, runs before features, returns None for void/no generator, refuses a client or wrong in-place level and batches above 4096. Nether roof is the scanned surface, not a cave floor.

Do NOT restore NativeTerrainColumn or an entry in the old JSON catalog. This needs a typed engine interface/API value (explicit Wire derive) and a server-side LevelRef method/host bridge with matching native and wasm behavior, an implementation in game/modding/native_world_columns.rs, generated catalog exposure and book docs. SDK catalog exports are protected by the SDK lane; coordinate that conversion with its owner. Existing types in horizon-mod-vocab/src/game.rs include Heightmap and ColumnBlocks, but neither models an unloaded generator sample. Design decisions needed: a new API-owned sample record with Wire; block representation (typed BlockState rather than old String) and biome representation; placement on the server world-generation/query trait versus existing world_edit trait; LevelRef access and wrong-level refusal semantics; typed error versus current String convention. Thus this is not mechanical reuse of existing plain return types. Under the parent ruling, omit the old entry point and file a typed capability follow-up rather than implementing those decisions inside the sync.

The new auto-merged game/native_world_columns/tests.rs was outside the original assigned list; parent explicitly authorized its removal. It is removed and included in deletions.txt: it imports the obsolete native_chunk_tickets test module and calls the absent old method. Move and adapt it with the eventual typed conversion. Its resource-dependent generator test verifies biome table choice, water/dry columns and no chunk loading; preserve those cases in converted coverage. Its second test covers teleport chunk holds and belongs with the upstream server-travel hold tests, not the API query module.

## Related auto merges

modding/native_scene.rs only changes a documentation ordinal; no runtime breakage found there. The new terrain test file above is the concrete auto-merge concern. The same upstream feature adds server-travel chunk holds, guest state and hold tests; these belong to the game/network workers, not this file set, and were not edited. No upstream Horizon product layer was removed by this resolution. No fragment directory was restored.

# Game conflict resolutions


- administration/commands/mod.rs: take upstream hold_for_chunks instead of direct trip removal; it clears the previous trip. Drop the incoming hub::teleport_cause call because the fork has no such function; that upstream cause-reporting feature is not taken. The fork's typed teleport decision remains intact.
- creatures/mob_group.rs: preserve upstream collision count across snapshot refresh and retain fork's shared.items argument to seen.
- creatures/mob_islands.rs: retain upstream own-random villager grouped parity test, then fork teleport-consumable test and its correct comment. Discard upstream raider comment that would incorrectly describe the fork test. Wire upstream helper-thread profiling around fork worker run and merge successful join sums, retaining lane cleanup guards and first-panic propagation.
- entities/boat_passengers.rs: remove obsolete mobs_near_carts optimization; upstream now selects candidates via CartCells in the new push path.
- entities/minecarts.rs: keep fork inlined CartIndex/cart_at and NBT, not upstream mod index/mod nbt declarations. Add only mod cells. Apply upstream one-walk grid/mounted-set optimization to canonical inlined index. Preserve rejected-passenger, callback-mutation, and TNT power tests. Append complete upstream ignored cart timing module independently, rather than allowing the conflict's shared closing braces to mix test modules.
- entities/minecarts/cells.rs: upstream new coarse-cell bitmap at canonical destination; no native API route.
- interface/advancements/mod.rs: reject reintroduced Toast::of_mod and native_advancement_display/AdvancementDisplay legacy native surface. No genuinely new gameplay feature in these conflicts; legacy APIs require a separate typed capability decision, not restoration.
- interface/chat.rs: reject reintroduced native_chat_input/native_set_chat_input legacy native route. No newly implemented chat behavior to port.
- interface/dedicated/mod.rs: retain fork runtime level cap plus upstream load_entities. Retain both fake-player management regression tests and upstream entities-settings parser test.
- world/tick_profile.rs: take upstream per-level/per-registry-id aggregation and helper-thread sum operations. Keep report-only sampling, excluding restored obsolete mod diagnostics sampling methods, storage and test. Preserve span-user accounting already present on fork. Retain helper-sum regression tests.
- world/server_travel.rs: three-way apply incoming delta to canonical fork source, retaining fork TravelCause hooks, missing respawn state, permission/callback decisions, owned LAN and controlled GuestList updates. Add hold state at every Trip initializer; hold/release updates use GuestList::update. Retain fork yaw/pitch handling; same-level holds neither reset body nor start portal cooldown. Canonical file uses self.lan, never upstream host argument API.
- world/server_travel/hold_tests.rs: port four upstream ignored tests to owned-LAN API and controlled guest updates. Explicitly place test guests at the teleport destination before creating holds: release_held_arrivals deliberately follows actual guest position, so an unchanged origin would cancel the hold immediately rather than test waiting.
- Delete stale game/server_travel.rs, game/server_travel/hold_tests.rs, game/minecarts/index.rs and game/minecarts/cells.rs after moving behavior into canonical files. No entities/minecarts/index.rs is needed: fork inlined implementation remains canonical.

## Integration checks and concerns

- Network sibling confirmed auto-merged Guest.tp_held field/default and restored MobSlot contract. These patches depend on that field, and on existing fork GuestList::update.
- Daily inventory carry: add the four world/server_travel/hold_tests.rs test names (all marked ignored with prepared-resource reasons) and entities/minecarts.rs cart_cost_bench::tick_carts_with_many_parked_carts (already upstream ignored timing). Lead owns the separate inventory carry. Villager grouped test is already ignored too; compare incoming inventory for its row.
- game.rs is outside the assigned list and remains conflicted in this worker tree; the requested zero extraction-remnant measurement must run on the integrated game.rs. No zero-remnant claim here.
- Auto-added old-path native files remain outside this export and must be rejected/converted by the native worker. In particular native_diagnostics.rs is the only incoming caller of the rejected mod sampling methods. Do not restore it to satisfy those references.
- Auto-added minecarts cells and server_travel hold tests were the related auto-merge breakages addressed here at canonical destinations. All other conflict areas remain untouched.
- Compilation and runtime behavior not checked. Static review should especially inspect held-arrival behavior together with network's tp_held packet handling.
# Network conflict resolutions

Inputs: fork 1a1b026ac73b086dcae413df50e5be8cc31d0e8b and upstream d1b6cd286ea4ebafd4e454829e199a867937b77a. Full bundle repeatedly failed unpacking; fetched the exact upstream commit directly and recreated no-commit merge on the exact fork base. No push or commit.

## Assigned paths

- lan/conn.rs: discard incoming obsolete native handshake, async pre-login and profile wrappers plus the orphan pre-login receiver helper. Resolved file equals fork version; does not revive the removed route.
- lan/hub/chat.rs: discard incoming native chat v1/v2 and decorate wrappers and duplicate NBT import. Existing fork transport and typed-host integration remain. Resolved file equals fork version.
- lan/entity.rs: retain fork Bodies (including vanilla particle bodies for non-Horizon peers), adopt parallel send thresholds and test switch. Crucially adapt BOTH changes and pairing in the new parallel path to Bodies, selecting by Viewer; upstream's raw body vectors would bypass fork particle compatibility. Remove undefined encode_all call in incoming path by using Bodies::encode.
- living_death.rs and sound_playback.rs: preserve fork deletion of obsolete native wrapper modules.
- mobs/seen.rs: preserve fork deletion: canonical fork snapshot stays in mobs.rs and takes Items, includes registry-aware invisibility/equipment behavior. Add new collision count initialization to that canonical snapshot instead.
- mobs.rs: retain canonical snapshot/pending push implementations and full fork sleeping/fox/creaking portal eligibility predicate; adopt DAB, collision cap fields, and restored MobSlot declaration. Do not add missing obsolete pending_pushes extraction module.
- mobs/ai.rs: discard obsolete native_register_goals wrapper. Adopt DAB scheduling and lobotomy brain wrapper with DamageLevel; retain fork panic-safe BorrowedGoalSelectors and typed Cx, using DAB selector tick. Retain selected_player semantics while adopting thread-local push target buffer. New collision-cap tests call canonical seen with Items.
- mobs/villager_brain.rs: expose restock_if_due to sibling lobotomy module, retain fork profession registry guard.
- save.rs: adopt upstream fallible raw/upgraded reads and saved-portal cache integration; retain trusted saved-fluid interning on successful raw region read. Writer extraction remains upstream's writes module; fork compression and backpressure remain in canonical save.rs/backpressure.rs.

## Authorized ancillary adjustments

- mobs/dab.rs and mobs/villager/lobotomy.rs: take DamageLevel rather than bare LegacyRandom and forward the same typed level to AI/brain.
- mobs/villager.rs: own-random brain optimization retains typed level and hook context. Its test now uses TestLevel, preserving its independent RNG comparison instead of calling typed brain with a bare RNG.
- health.rs: add reborrow_with_random with every non-RNG field identical to existing reborrow, substituting the temporary RNG by borrowing rather than swapping shared state. Attachments, hooks, attack, client side, and knockback entry remain intact, including unwind behavior.
- mobs/creaking.rs: remove auto-added weaker duplicate mob_can_use_portal; canonical mobs.rs checks sleeping and fox sleep too.
- mobs/mob_slot.rs: restore upstream's non-hook MobSlot abstraction and both impls verbatim. Contract sent to game worker; game mob_group/mob_islands require this.

## Auto-merge audit

- save/writes.rs start_writer is text-identical to previous fork implementation; writer queue extraction preserves existing compression and sequencing. New portal cache invalidates after pending write publication and uses fallible reads so failures are not cached as absence. No additional writer change needed.
- saved_portals.rs resource test remains for integrator's promised daily-ignore follow-up.
- LAN guest.rs adds tp_held default false, consistent with game's requested travel adaptation. Contract sent to game worker. LAN waypoints changes only restructure per-receiver computation; no obsolete native API found there.
- mobs/living.rs collision shape refactor preserves SceneCollisions and context-aware shape query. mobs/world.rs adds reusable per-thread target buffer; selected-player fork access retained at its AI caller.

## Verification and limits

# Other conflict resolutions


- crates/horizon-items/src/item_data.rs: take ItemStack::from_nbt as a thin engine-owned wrapper of existing item_nbt parsing. Drop obsolete API-level stamp. It does not expose engine types through mod_api.
- crates/horizon-server-launch/src/config.rs: combine independent biome_regions and entities sections, fields, serialization list and tests. Keep fork's extracted config owner; take all incoming entity settings, defaults and validation. Neither feature displaces the other.
- crates/horizon-server/src/engine.rs: preserve horizon_server_launch::command as canonical process construction. Add upstream entity environment settings and pass Entities through spawn. Reuse the shared command rather than resurrect duplicate resource discovery and control_path implementations. Incoming resource discovery tests belong to the already-extracted launch crate and are not copied back into engine.rs; incoming command/environment tests are kept. Transfers are prepended to the shared command arguments so --transfers remains before --dedicated, satisfying the incoming ordering contract while retaining the shared process configuration.
- launcher/src/native_mods/api.rs: keep fork template-driven main generation, typed point inventory and SDK build contract. Result is byte-identical to main. Incoming allocator-source probing and handwritten old mod_api executable generation target the obsolete native build route; the fork template already owns the allocator declaration, so no compatibility fallback is added. No protected SDK layout or catalog export edits.

## Related auto-merge inspection

- supervisor.rs correctly adds &config.entities at the sole engine::spawn call; no edit needed.
- horizon-threads/src/lib.rs only makes serial_tick_all public; no allocator rewrite was incoming there. The rejected launcher allocator patch is not needed for this fork.
- server_perf/world.rs raises entity capture row capacity to 512; no stale native API use.
- The global merge remains conflicted outside this area, untouched. In particular the new terrain column capability must be covered by native area review, not by restoring the old game.rs exports.

Compile and runtime behavior are not checked; CI remains the integration proof.

## Integration correction

The initial transfer-order implementation mistakenly called nonexistent Command::args_clear (caught by integrator compile-only check). Replaced it with shared command_with_transfers construction, which appends transfer options before --dedicated. The ordinary command entry delegates with None, preserving existing callers without rebuilding a Command. Ancillary full file crates/horizon-server-launch/src/lib.rs is now exported alongside the corrected engine.rs. Rustfmt and scoped diff whitespace guard passed; no compile or tests run for the correction.

## Extraction measurement
Zero genuine remnants. The nine retained name-only clashes are render_with, in_own_process, level, key, all, stage, sea_level, native, exists (983 function lines). game.rs is unchanged from the fork base, 12248 physical lines by the measurement tool.

## Current-main integration

Merged fork main 959c0df470e (including the SDK stack) without conflicts. Main's generated catalog export, prebuilt package layout, launcher game/holder routing and workflow changes are retained; no upstream compatibility route replaces them. The upstream pin remains d1b6cd286ea4ebafd4e454829e199a867937b77a.

Helper-thread profiling call sites are retained in creatures/mob_islands.rs. The fork builds that loop only with `upstream-parallel-mobs`, so sampling/Sums/helper merge functions use the same feature condition (also available to their unit tests). The unused legacy mod span bit is test-only for span-user arbitration coverage; obsolete native diagnostics remain absent.
