# Sync 32 decisions

## Pin and scope

- Initial fork base: `396e1d85add` (main after the formatting-only #3545). Fresh main rollup 49 at `03d2abebc27ae496e833992841fd8fe9de47416c` was merged before the first push; its paths do not overlap the seven sync code/tool paths.
- Upstream pin: `ba91f75cb21a74026319d1393c17fc5061dbd725`.
- Pin evidence: upstream `ci.yml` push-to-main run `43197652294` completed successfully, updated 2026-11-12 10:25:09 UTC. The newer upstream head `f24ee1accf1` still had its own run in progress when this sync began; it is not stacked onto this sync.
- Seven incoming paths. The work is chest-boat pending loot, slot access and destroying removals, plus its command corpus. No launcher, CI-policy, mod-point machinery or slow-shard repair changes are included.

## Resolution and semantic sweep

| Area | Decision |
| --- | --- |
| Boat loot owner | Keep one `boat_loot_pending` and one `unpack_boat_loot` implementation in the canonical boat module. Port the fork's typed list mutation through `boats.update` and pass the existing item-default registry to resource-backed loot loading. |
| Menu and automation | Menu opens pass `player = true`; automation and destroying removals pass false. Only player opening awards the loot criterion, before filling. The boat helper reacquires its entity index after advancement rewards and synchronizes the open placeholder container. The menu refuses if those rewards removed the original boat, rather than reading a stale index. |
| Commands and destruction | Preserve `kill_vehicle_object`: reentrant-removal protection, real dismount callbacks, retained removed vehicles and entity cleanup stay owned there. Add pending-loot unpacking to its boat branch. Do not add upstream's parallel raw `kill_boat` removal path. This is fork behavior kept, not an omitted destruction behavior. |
| Slot authority | Synchronize the menu's authoritative items before rolling. Fill only empty slots, retain occupied stacks, consume the table once, and update the open menu copy after rolling. |
| Dead file | Keep the retired `game/lan_actions.rs` deleted. Its changed caller is ported into `network/lan_actions/menus.rs`; its removed loot helpers are removed from that canonical file too, so neither stale duplicates nor an orphan monolith remain. |
| Typed values | Use the core NBT type and the existing registry/item-default owners. Do not restore mutable indexing into the typed vehicle list. |

Diverges from upstream: canonical relocated modules, typed vehicle ownership and the fork's single destruction/dismount owner are preserved. Upstream's loot timing and outcomes are retained through those owners.

## Incoming tests and resource gating

Both new test declarations are retained, with all upstream assertions and numerical expectations unchanged:

| Test | Canonical location | Resource boundary |
| --- | --- | --- |
| `chest_boat_automation_does_not_award_player_loot` | `game/blocks/machines/mod.rs::hopper_cart_transfer_tests` | Existing ignored `HORIZON_TEST_RESOURCES` gate; use the fork's real `native_entity_refs::tests::with_game` fixture instead of the absent upstream `native_machines::tests::server_game`. |
| `chest_boat_slot_access_fills_only_empty_slots_once` | Same module | Same prepared-resource gate and real fixture; retains authoritative open-menu, exact seed-73 loot, occupied-slot, spent-table and no-reroll assertions. |

The incoming advancement overlay fixture uses the existing `PackOverlay::NONE` remainder pattern to supply typed vanilla load conditions for its unconditional overlay files. The fixture opens the actual typed world lifecycle and uses real resource loading and container paths. No stub inventory, fabricated production result or extra skip is introduced. These ignored assertions have not been run locally; compile-only checking and a green ordinary PR gate do not prove them.

## Tools and other areas

`boat_loot_corpus.py` exercises real hopper ticks and `/kill` or `/damage` for oak chest boats and bamboo chest rafts with two seeds. The existing command corpus registers it. The commands use frozen ticks, real force-loaded columns and observed items; this sync does not run the corpus locally. No launcher files or hook-fragment files are incoming.

## Verification

Only rustfmt and the permitted compile-only workspace/all-target check are used locally. No tests, clippy, harnesses or game runs are claimed. Exact compile and CI results belong in the PR's self-check block.
