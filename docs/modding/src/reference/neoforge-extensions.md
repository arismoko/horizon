# NeoForge extension methods and Horizon

This page compares the public methods of NeoForge 26.3's object-extension interfaces with what a Horizon mod can
answer today. It is the reference the Parts lane starts from: before proposing a new block or item hook, check
whether content data, a handle or an existing Part already answers the same question.

Read from NeoForge `26.3.0.101` (`neoforge-26.3.0.101-universal.jar`, `net.neoforged.neoforge.common.extensions`)
with `javap -p`, and from Horizon at commit `061a06c7b`. Method lists and parameter names are factual citations; no
NeoForge source or Javadoc is copied. Counts exclude each interface's private `self()` accessor.

## How to read the verdicts

| Verdict | Meaning |
| --- | --- |
| **Covered** | A mod can answer the same question for its own object, at the same moment, through content data, a handle or a Part method. |
| **Partial** | The game lets a mod answer something close, but one of these differs: the object (a global hook instead of a per-block/item method), the moment, the arguments, or the answer (a factor or notice where NeoForge returns a decision). |
| **Missing** | Neither data, a handle nor a hook exposes the question. |
| **n/a** | The method has no Horizon equivalent because the architecture removes its reason to exist. |

A **Part method** is a generated method on `BlockPart`, `ItemPart`, `BlockEntityPart`, `MenuPart` or `EntityPart`,
declared with `scope` in `crates/horizon-mod-vocab/src/hooks/<area>.rs`. A Part method runs only for the objects a
mod attached its part to. An **unscoped hook** in the same files is global: every mod hears it and filters by the
arguments itself. **Data** means a content declaration in `crates/horizon-mod-package/src/content.rs`, read into
the game through `crates/horizon-items/src/blocks.rs`. **Handle** means a value or reference type in
`crates/horizon-mod-vocab/src/` (`ItemStack`, `ItemResource`, `AttachmentStore`).

`EntityPart` exists today with goal and behavior registration, `spawn_cluster_size`, and `is_shearable` / `on_sheared` (`docs/modding/src/concepts/trait-mods/023-entity-parts-and-mob-goals.md`).
It does not answer `IEntityExtension` methods; those are covered only where a global hook or a handle does.

## Counts

<!-- extension-counts -->

The book build computes this table from the method verdicts below; there is no snapshot to update.

Of the 60 block methods, 57 exist a second time with the state as receiver, so the state-extension counts are
the block counts minus three; see [`IBlockStateExtension`](#iblockstateextension). The block follow-up is
[#1038](https://github.com/arismoko/horizon/issues/1038) and the item follow-up is
[#1039](https://github.com/arismoko/horizon/issues/1039). Their groups were written from the earlier audit and
do not name every Missing row yet. Both predate hooks that have since landed, so some of their rows are narrower
or already answered; the last section lists what changed.

The earlier audit's counts included `self()` and folded the four `isInFluidType` overloads into one; this page
counts each declared public method, so the entity total is 33 rather than 30.

## `IBlockExtension`

Data fields are static values from a block's content declaration, inherited from its template when unset. They
answer the method for a mod block but cannot vary by level, position or entity. The engine's own copy of the
same facts is `StateFacts` in `crates/horizon-items/src/blocks.rs`.

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `getFriction(state, level, pos, entity)` | Covered | `BlockPart::friction` (`hooks/friction.rs`) | Asked where an entity on the block reads its friction, with the entity's id and type and the position, from the block's own value; an answer that is not finite or is outside `0..=2` is dropped. Players, mobs, armor stands, items, experience orbs, boats and leads. The part answers from its inputs, without the level. |
| `hasDynamicLightEmission(state)` | Covered | `BlockProperties.dynamic_light` → `StateFacts.dynamic_light` | Content data, per block, not per state: it sits on the block's row, so every state of a declared block is asked and a block with mixed states pays for all of them. An undeclared block is never asked, where NeoForge's declaration only gates a chunk's first lighting. |
| `getLightEmission(state, level, pos)` | Covered | `BlockPart::light_emission` (`hooks/light_emission.rs`), for a block declaring `dynamic_light` | Asked with the position as a chunk comes in, as an edit sets the block and at `LevelRef::check_light` (`LevelLightEngine.checkBlock`), where vanilla's engine reads emission again, only on the thread that owns the level. A client also asks again for the sections a server's light update changed, at most once per cell per update, a Horizon addition. Not asked for the block's own rendered light, ambient occlusion or the spawn rule, which take the state's light, nor from a light worker. The answer is clamped to 15 at the engine, where NeoForge clamps only its light manager's stored value. Without `dynamic_light`, `BlockProperties.light` is static. |
| `ignitedByLava(state, level, pos, direction)` | Covered | `BlockPart::ignited_by_lava` (`hooks/ignited_by_lava.rs`) | Asked from the face toward the air lava would light, where `LavaFluid.isFlammable` reads it; with no answer the template's `IGNITED_BY_LAVA` state flag (`StateFacts`) decides. No content field sets it. |
| `isLadder(state, level, pos, entity)` | Covered | `BlockPart::is_ladder` (`hooks/is_ladder.rs`) | Asked of the block a living entity stands in, with the entity id, after the `#climbable` tag: the part's answer replaces the tag's, either way. Players and mobs; armor stands never ask. |
| `makesOpenTrapdoorAboveClimbable(state, level, pos, stateAbove)` | Missing | — | Engine rule in the climbing code; not exposed. |
| `isBurning(state, level, pos)` | Missing | — | No per-block burning query; `block_burn` is fire removing a block. |
| `canHarvestBlock(state, level, pos, player)` | Covered | `BlockPart::can_harvest_block` (`hooks/can_harvest_block.rs`) | Asked where `player_harvest_check` fires, before it: the global hook still hears and may change the part's answer. |
| `onDestroyedByPlayer(state, level, pos, player, stack, willHarvest, fluid)` | Partial | `BlockPart::on_destroyed_by_player` (`hooks/on_destroyed_by_player.rs`) | Asked in the player's break after `break_block` allowed it, before anything of the break changes the world, with the tool and whether the break harvests. The part answers whether the block was destroyed (`Some(true)`: it removed the block itself, `Some(false)`: it stays) in place of all of `playerWillDestroy` (pot cracking, piglin anger, tripwire disarming, a creative shulker box's or beehive's drop) and the removal, and `None` keeps vanilla's. `willHarvest` is the tool's own answer; the mods' harvest listeners are asked once after the removal, of the adjusted state and the stack in the hand then, as vanilla does. The fluid is not an argument. |
| `onDestroyedByPushReaction(state, level, pos, direction, fluid)` | Missing | — | No hook at the piston/fluid destruction point. |
| `getRespawnPosition(state, entityType, level, pos, angle)` | Covered | `BlockPart::respawn_position` (`hooks/respawn_position.rs`) | Asked on the server as the respawn point is resolved (and as a player leaves the End), starting from the bed's, anchor's or forced point's spot; `None` sends the player to the world spawn. Asked for any block, where NeoForge asks only beds; an anchor loses a charge only when the answer is a spot. The global `player_respawn_position` still hears the result. |
| `getExplosionResistance(state, level, pos, explosion)` | Partial | `BlockProperties.explosion_resistance` → `StateFacts.explosion_resistance` | Static; the explosion is not consulted. |
| `getCloneItemStack(level, pos, state, includeData, player)` | Covered | `BlockPart::clone_item_stack` (`hooks/clone_item_stack.rs`) | `include_data` and the player are passed; leaving the stack unset keeps the game's item. |
| `addLandingEffects(state, level, pos, stateBelow, entity, count)` | Missing | — | `fall_on` edits fall damage, not the landing particle/sound burst. |
| `addRunningEffects(state, level, pos, entity)` | Missing | — | No hook for the running dust burst. |
| `playFallSound(state, level, pos, entity)` | Partial | `BlockPart::sound_type` (`hooks/sound_type.rs`, `cause` `Fall`) | Picks the event, volume and pitch of the one fall sound the game plays; the part can't play or suppress its own. Asked for mobs; a player's block fall sound is not played yet ([#3178](https://github.com/arismoko/horizon/issues/3178)). |
| `playStepSound(state, level, pos, entity, volume, pitch)` | Partial | `BlockPart::sound_type` (`hooks/sound_type.rs`, `cause` `Step`) | Picks the event, volume and pitch of the one step sound the game plays; the part can't play or suppress its own. |
| `canSustainPlant(state, level, pos, direction, plantState)` | Covered | `BlockPart::can_sustain_plant` (`hooks/can_sustain_plant.rs`) | The soil's answer for the plant above it (allow, deny or none), asked before the plant's `may_place_on`, which still answers for your plant. The direction is always up. |
| `onTreeGrow(state, level, posConsumer, random, pos, feature)` | Covered | `BlockPart::on_tree_grow` (`hooks/on_tree_grow.rs`) | Asked of the soil under each trunk column a trunk placer starts, in worldgen and when a sapling grows, with the tree's random: answer a state, none, or leave it to the tree's provider. The first worldgen Part method: no `Host`, a `WorldgenHost`. `feature` is left out: an inline tree feature has no key. |
| `isFertile(state, level, pos)` | Missing | — | No fertile-block query; bonemeal hooks fire later. |
| `getExpDrop(state, level, pos, blockEntity, breaker, tool)` | Partial | `break_block.exp_to_drop` (`hooks/base.rs`) | Replaces the XP sample for a player break; global, not per-block, and the block entity/tool/breaker are not arguments. [#1038](https://github.com/arismoko/horizon/issues/1038). |
| `rotate(state, level, pos, rotation)` | Missing | — | Structure rotation is engine behaviour; no callback. |
| `getEnchantPowerBonus(state, level, pos)` | Missing | — | Bookshelf enchanting power is a hard-coded block rule. |
| `onNeighborChange(state, level, pos, fromPos)` | Missing | — | `neighbor_changed` is `Block.neighborChanged`, a different callback. [#1038](https://github.com/arismoko/horizon/issues/1038). |
| `shouldCheckWeakPower(state, level, pos, direction)` | Covered | `BlockPart::should_check_weak_power` (`hooks/should_check_weak_power.rs`) | The argument direction is the input side, as NeoForge's. |
| `getWeakChanges(state, level, pos)` | Missing | — | No weak-change observer query. [#1038](https://github.com/arismoko/horizon/issues/1038). |
| `getSoundType(state, level, pos, entity)` | Covered | `BlockPart::sound_type` (`hooks/sound_type.rs`) | Asked with the position, the entity (nullable: the stepping, landing, placing or digging one, none for a break or a dispenser's placement) and the sound about to play, on NeoForge's paths only: the step and combination step, a horse's step (at the block below a snow layer), a mob's fall, a block item's placement, the break event and a sniffer's digging. Not for the mining hit sound (its own client hook, `BlockPart::play_hit_sound`, below) or sculk growth; an answer with a volume or pitch that is not finite and at least 0, or an event that is not a `namespace:path` id, is dropped. The part answers from its inputs, without the level; `host.block_states().sound(state)` still reads the static type. |
| `getBeaconColorMultiplier(state, level, pos, beaconPos)` | Missing | — | Beacon beam colour is engine-owned. |
| `getStateAtViewpoint(state, level, pos, viewpoint)` | Missing | — | No viewpoint-state query. |
| `getBlockPathType(state, level, pos, mob)` | Partial | `BlockPart::is_pathfindable` (`hooks/is_pathfindable.rs`) | Answers whether a `PathComputationType` may pass, once per state and type; not a `PathType` per mob or position. [#1038](https://github.com/arismoko/horizon/issues/1038). |
| `getAdjacentBlockPathType(state, level, pos, mob, pathType)` | Missing | — | No adjacent-path-type query. |
| `isSlimeBlock(state)` | Covered | `BlockPart::is_slime_block` (`hooks/block_sticky.rs`) | Server only, asked by the piston structure resolver for its slime-beside-honey rule. A piston moving a slime block still bounces the entities it pushes off vanilla's slime block alone. |
| `isStickyBlock(state)` | Covered | `BlockPart::is_sticky_block` (`hooks/block_sticky.rs`) | Server only, asked by the piston structure resolver; starts with vanilla's slime and honey. |
| `canStickTo(state, otherState)` | Covered | `BlockPart::can_stick_to` (`hooks/block_sticky.rs`) | The part of `state` answers; the resolver asks both sides and needs both to say yes. |
| `isFlammable(state, level, pos, direction)` | Covered | `BlockPart::fire_odds` (`hooks/fire_odds.rs`) | Flammable is ignite odds above zero, as `FireBlock.canBurn` reads it, so the odds answer it for each position and face; no separate predicate. |
| `getFireSpreadSpeed(state, level, pos, direction)` | Covered | `BlockPart::fire_odds` (`hooks/fire_odds.rs`) | Ignite/spread odds (the first element of `fire_odds`'s `(ignite, burn)` tuple) replace vanilla's table, with the block's position and the face toward the fire. |
| `getFlammability(state, level, pos, direction)` | Covered | `BlockPart::fire_odds` (`hooks/fire_odds.rs`) | Burn/destruction odds (the second element of `fire_odds`'s `(ignite, burn)` tuple) replace vanilla's table, with the block's position and the face toward the fire. |
| `onCaughtFire(state, level, pos, direction, igniter, stack)` | Partial | `BlockPart::on_caught_fire` (`hooks/on_caught_fire.rs`) | A notice after the burn roll and spread/burn/ignite cancel gates, before replacing the block, with its old state and face toward the fire. Writes at that position are overwritten; TNT primes afterward. Fire has no igniter and an empty stack. Lava, lightning, spread into air, flint and steel, TNT use, burning projectiles, redstone and dispensers do not fire this spread-only hook. |
| `isFireSource(state, level, pos, direction)` | Covered | `BlockPart::is_fire_source` (`hooks/is_fire_source.rs`) | Asked of the block under a fire on each fire tick, before the dimension's `infiniburn` tag: yes makes the fire endless, no refuses a tagged block. The direction is always up. |
| `canEntityDestroy(state, level, pos, entity)` | Missing | — | Mob-grief checks are engine rules; `entity_mob_griefing` is a global event at a different moment. |
| `canDropFromExplosion(state, level, pos, explosion)` | Partial | `explosion_detonate` (`hooks/base.rs`) | Removes positions from the blast before blocks are hit; no per-block keep/drop query. |
| `onBlockExploded(state, level, pos, explosion)` | Partial | `explosion_detonate` (`hooks/base.rs`) | Same global pre-blast hook; no per-block callback after the block is removed. [#1038](https://github.com/arismoko/horizon/issues/1038). |
| `collisionExtendsVertically(state, level, pos, entity)` | Missing | — | Collision shapes are data; no per-entity extension query. |
| `shouldDisplayFluidOverlay(state, level, pos, fluid)` | Missing | — | Overlay drawing is engine-owned. |
| `getToolModifiedState(state, context, ability, simulate)` | Covered | `BlockPart::tool_modified_state` (`hooks/tool_modified_state.rs`) | Asked of the clicked block before the global `block_tool_modification`, with the vanilla result: the part allows, replaces or, cancelled, denies the action. `simulate` is `ToolUse::simulated`. Axe, shovel and hoe actions and a shovel dousing a campfire. |
| `isScaffolding(state, level, pos, entity)` | Missing | — | Scaffolding is identified by block class; no content field or Part. |
| `hidesNeighborFace(level, pos, state, neighborState, direction)` | Missing | — | `block_render_visibility` hides a whole state from the mesher, not one face. |
| `supportsExternalFaceHiding(state)` | Missing | — | No per-face culling query. |
| `onBlockStateChange(level, pos, oldState, newState)` | Partial | `block_changed` (`hooks/base.rs`, a global server hook) | Fires after the commit but before block side effects and neighbour updates, and only on an authoritative server. It may not modify the level. |
| `canBeHydrated(state, level, pos, fluid)` | Missing | — | Farmland hydration is engine-owned; fluid `canHydrate` is data. |
| `getMapColor(state, level, pos, defaultColor)` | Partial | `BlockProperties.map_color` → `StateFacts.map_color` | Static; no position. |
| `getAppearance(state, level, pos, direction, neighborState, neighborPos)` | Missing | — | No appearance query. |
| `getPistonPushReaction(state)` | Covered | `BlockPart::piston_push_reaction`, `BlockProperties.push_reaction` | State-only override cached on both sides at load and reload; `None` preserves properties. |
| `isEmpty(state)` | Missing | — | Air/replaceable facts exist (`block_states().replaceable`), but no `isEmpty` answer for a state. |
| `getBubbleColumnDirection(state)` | Missing | — | Bubble-column rules are engine-owned. |
| `shouldHideAdjacentFluidFace(state, direction, fluid)` | Missing | — | Fluid-face culling is engine-owned. |
| `getBounceRestitution(level, pos, state, entity)` | Covered | `BlockPart::bounce_restitution` (`hooks/bounce_restitution.rs`) | Asked for the player and the mobs on both sides, with the entity's level as a read-only `LevelSide` host, the entity, its type and the block's position. The answer must be pure because replayed ticks ask again: the same host and event inputs must give the same factor. Dropped items keep their own slime rule. `BlockPart::bounce_on` still edits the movement after the bounce. |
| `getRelocability(level, pos, state)` | Missing | — | No relocation query (pistons, structure blocks). |
| `canBeConnectedTo(state, direction, neighborBlock)` | Partial | `BlockPart::connects_to` (`hooks/connects_to.rs`) | Asks the connecting fence, wall, bars or pane (your block), not the block being connected to: a modded target cannot answer for a vanilla fence, and only those shapes ask at all. |
| `getModelData(state, level, pos)` | Missing | — | No `ModelData`; block-entity drawing takes a `block_entity_submit` part with its own model path. |

## `IBlockStateExtension`

This interface repeats the block questions with the state as receiver instead of the first argument. All 57
methods are the same behaviours and take the same verdict as the `IBlockExtension` row of the same name, with
the parameters shifted by one. Three block methods have no state-receiver form:

| Block-only method | Verdict there |
| --- | --- |
| `getPistonPushReaction(state)` | Covered (`BlockPart::piston_push_reaction`, content `push_reaction`) |
| `getMapColor(state, level, pos, defaultColor)` | Partial (content `map_color`) |
| `makesOpenTrapdoorAboveClimbable(state, level, pos, stateAbove)` | Missing |

The handful of signature differences (`canStickTo` takes
the neighbour state; `getCloneItemStack` reorders its arguments; `getEnchantPowerBonus` takes a `LevelReader`)
do not change any verdict.

## `IItemExtension`

Item data is a 26.3 component patch over a vanilla template (`crates/horizon-mod-package/src/content.rs`),
which is the same place NeoForge's item components live. Reads are on `host` (`host/items.rs`) or on the owned
`ItemStack` / `ItemResource` values.

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `getDefaultAttributeModifiers(stack)` | Partial | `item_attribute_modifier` (`hooks/item_attribute_modifier.rs`) | Global `ItemAttributeModifierEvent` edits the stack's modifiers; the stack filters it, but it is not a Part method. |
| `onDroppedByPlayer(stack, player)` | Partial | `item_toss` (`hooks/item_toss.rs`) | Global `ItemTossEvent`, cancellable; the item is not a Part attachment. |
| `getHighlightTip(stack, name)` | Missing | — | No highlight-name hook. |
| `onItemUseFirst(stack, context)` | Covered | `ItemPart::on_item_use_first` (`hooks/on_item_use_first.rs`) | Runs after `use_item_on_block` let it through, before the block's own use. |
| `isPiglinCurrency(stack)` | Covered | `ItemPart::is_piglin_currency` (`hooks/is_piglin_currency.rs`) | Starts from vanilla's answer, which is the item `minecraft:gold_ingot` (`PiglinAi.BARTERING_ITEM`; 26.3 has no currency tag), and is asked where vanilla asks: a piglin wanting a stack on the ground, an adult taking one from the hand (and the client's prediction of that click), and the end of the admiring, where currency is bartered or lost to a hit. A ground stack is admired only when it is also `#piglin_loved`. Both sides. [#3226](https://github.com/arismoko/horizon/issues/3226). |
| `makesPiglinsNeutral(stack, entity)` | Covered | `ItemPart::makes_piglins_neutral` (`hooks/makes_piglins_neutral.rs`) | Starts from `#minecraft:piglin_safe_armor` (which content mods can already add to) for each stack in a player's armour slots, with the wearer, and may make a piece count or refuse a tagged one. Server-side. [#3226](https://github.com/arismoko/horizon/issues/3226). |
| `isCombineRepairable(stack)` | Partial | `minecraft:repairable` component via `ItemResource::repairable` | Data answers which material repairs it; the anvil combine predicate is not exposed. |
| `getXpRepairRatio(stack)` | Covered | `ItemPart::xp_repair_ratio` (`hooks/xp_repair_ratio.rs`) | Asked in Mending's repair (`ExperienceOrb.repairPlayerItems`) for the picked item, with the ratio arriving as its `minecraft:repair_with_xp` repair per point. Server-side, read-only `Host`. |
| `onStopUsing(stack, entity, timeLeft)` | Partial | `ItemPart::release_using` (`hooks/base.rs`) | Runs on an early release only, after the vanilla item's release; finishing the use or switching to another item stops it without asking. Both sides. |
| `onLeftClickEntity(stack, player, entity)` | Covered | `ItemPart::on_left_click_entity` (`hooks/item_attack.rs`) | Runs where `Player.attack` starts, after `attack_entity` let the blow through; a cancel leaves the target unhurt, with no `hurt_enemy` and no wear. |
| `getCraftingRemainder(instance)` | Covered | `ItemPart::crafting_remainder`, content `crafting_remainder` | The per-stack Part override replaces the data remainder in crafting, fuel and quick moves. |
| `getEntityLifespan(stack, level)` | Partial | `item_expire` (`hooks/base.rs`) | Adds extra ticks to a dropped item's life; no absolute lifespan and not per-item. |
| `hasCustomEntity(stack)` | Missing | — | No custom dropped-entity selection. |
| `createEntity(level, entity, stack)` | Missing | — | Same. |
| `onEntityItemUpdate(stack, itemEntity)` | Missing | — | No per-item dropped-entity tick. [#1039](https://github.com/arismoko/horizon/issues/1039). |
| `doesSneakBypassUse(stack, level, pos, player)` | Missing | — | No sneak-bypass query. |
| `canEquip(stack, slot, entity)` | Covered | `ItemPart::can_equip` (`hooks/can_equip.rs`) | Starts as the stack's own `equippable` component's answer (slot and `allowed_entities`); the part sets it either way, with a read-only `Host` and the entity as a `LivingRef`. Asked for the player's armour slots (both menus), a dispenser's equip and a mob's pickup. A wasm mod is given a copy of the stack on each call (#3549 tracks the prepared path). |
| `getEquipmentSlot(stack)` | Covered | `ItemResource::equippable_slot`, `host.item_equippable_slot` | Read from the effective `equippable` component. |
| `onEntitySwing(stack, entity, hand)` | Covered | `ItemPart::on_entity_swing` (`hooks/item_attack.rs`) | Runs where `LivingEntity.swing` starts, on both sides; a cancel starts no swing (no animation, no packet). |
| `getDamage(stack)` | Covered | `ItemStack::damage` | The stack's numeric damage override. |
| `getMaxDamage(stack)` | Partial | `minecraft:max_damage` effective component, `host.max_damage` | The stack's effective component includes a patch over the item default; `host.max_damage` reads that default only. |
| `isDamaged(stack)` | Covered | `ItemStack::damage`, `damage_component` | Distinguishes explicit zero from absent. |
| `setDamage(stack, damage)` | Partial | `ItemStack::set_damage` | Stores the requested value; the game checks it against the effective maximum when it accepts the write instead of clamping here. |
| `canPerformAction(instance, ability)` | Partial | `ItemPart::can_perform_action` (`hooks/can_perform_action.rs`), `block_tool_modification` (`hooks/block_tool_modification.rs`) | The part answers per stack and per namespaced ability id, starting from vanilla's answer, where vanilla checks the item kind: `minecraft:shield_block` (a raised item blocking attacks, its use time and animation), `minecraft:shears_harvest` (sheep, mooshroom, snow golem, bogged, bee nests, a dispenser's shears) and `minecraft:shears_carve` (a pumpkin). The block side of a tool action (strip, wax, path, ...) is `block_tool_modification`; no tool ability (`axe_strip`...) is asked of the item yet. [#1039](https://github.com/arismoko/horizon/issues/1039). |
| `getMaxStackSize(stack)` | Partial | `max_stack_size` component, `host.max_stack_size` | The stack's effective component includes a patch over the item default; `host.max_stack_size` reads that default only, and invalid values are refused at the write. |
| `isPrimaryItemFor(stack, enchantment)` | Partial | `host.enchantment_supports` | Answers supported items; the narrower "primary item" distinction is not exposed. |
| `supportsEnchantment(stack, enchantment)` | Partial | `host.enchantment_supports` | A client-side read of `Enchantment.isSupportedItem` for an item id; not per stack and without NeoForge's enchanted-book special case. |
| `getEnchantmentLevel(instance, enchantment)` | Covered | `get_enchantment_level` (`hooks/get_enchantment_level.rs`) | Edits the level gameplay sees. |
| `getAllEnchantments(stack, lookup)` | Covered | `get_enchantment_level` with no target | The hook's `target` is `None` for every-enchantment reads. |
| `shouldCauseReequipAnimation(stack, oldStack, equipped)` | Missing | — | Client animation rule; no hook. |
| `shouldCauseBlockBreakReset(oldStack, newStack)` | Missing | — | Client mining-reset rule; no hook. |
| `canContinueUsing(stack, otherStack)` | Missing | — | No continue-use query. |
| `getCreatorModId(registries, stack)` | Covered | `ItemPart::creator_mod_id` (`hooks/creator_mod_id.rs`), read by `ItemResource::creator_mod_id` | Both sides, from the stack alone. The answer arrives as the item id's namespace; a part sets another mod or none. An item without a part costs one table lookup. |
| `onAnimalArmorTick(stack, level, mob)` | Missing | — | No per-item animal-armour tick. |
| `damageItem(stack, amount, entity, onBreak)` | Covered | `ItemPart::damage_item` (`hooks/damage_item.rs`) | Runs on the server for the parts of the worn item, after Unbreaking, on every wear path (a player's tools, armour and shield, a dispenser's tool, a mob's armour). The part gets the stack and the amount about to be lost, and changes it (0 or less leaves the item whole) or cancels the wear. It is not given the wearing entity (`host.players().acting_player()` names the player on a player's paths) and has no `onBreak` consumer: the break is `player_destroy_item`. |
| `onDestroyed(itemEntity, source)` | Missing | — | `item_despawn` is lifetime removal, not damage destruction. |
| `isGazeDisguise(stack, player, target)` | Missing | — | No gaze-disguise query. |
| `canWalkOnPowderedSnow(stack, entity)` | Missing | — | Powder-snow walking is engine-owned. |
| `isDamageable(stack)` | Covered | `ItemResource::damageable(host)` | Reads the effective components: `minecraft:max_damage` and `minecraft:damage` present, `minecraft:unbreakable` absent. |
| `getSweepHitBox(stack, player, target)` | Missing | — | `sweep_attack` decides whether a sweep happens, not its box. |
| `isNotReplaceableByPickAction(stack, player, slot)` | Missing | — | Pick-block replacement is engine-owned. |
| `canGrindstoneRepair(stack)` | Missing | — | `grindstone_place` observes or edits the result; no per-item predicate. |
| `canBeHurtBy(stack, source)` | Missing | — | No item damage-immunity query. |
| `applyEnchantments(stack, enchantments)` | Missing | — | No item-level enchantment application hook. |
| `canFitInsideContainerItems(stack)` | Partial | `MenuPart::menu_slot_rules` (`hooks/base.rs`) | Slot rules implement container-item restrictions (a shulker box in a slot); no per-stack override. |
| `onGlideDamage(stack, entity, slot)` | Missing | — | No per-item glide-damage callback. |

## `IEntityExtension`

`EntityPart` carries no `IEntityExtension` method (its methods are goals, spawn group size and shearing), so
entity coverage is data, handles and global hooks. On the server a mod can act on entities through `EntityRef` and `AttachmentStore`.

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `captureDrops()` | Missing | — | `living_drops` edits a dying mob's drops; it does not capture an arbitrary entity's drops. |
| `captureDrops(collection)` | Missing | — | Same. |
| `getPersistentData()` | Covered | `AttachmentStore`, `AttachmentHolder::Entity` | Saved and synced attachments replace the writable compound; the data is typed, not raw NBT. |
| `shouldRiderSit()` | Missing | — | Rider pose is engine-owned. |
| `canRiderInteract()` | Missing | — | No rider-interaction query. |
| `canBeRiddenUnderFluidType(type, rider)` | Missing | — | `entity_mount` fires on mount/dismount, not on fluid riding. |
| `canTrample(level, state, pos, distance)` | Partial | `farmland_trample` (`hooks/farmland_trample.rs`) | Global `BlockEvent.FarmlandTrampleEvent`; per-entity override is not exposed. |
| `getClassification(includeVehicles)` | Partial | `host.entity_classification`, content entity type | A read of the template's category; a mod entity cannot set a different one per entity. |
| `isAddedToLevel()` | Partial | `entity_join_level` / `entity_leave_level`, `host.entity_added_to_level` | State is tracked by the game; the handles tell a mod after the fact. |
| `onAddedToLevel()` | Partial | `entity_join_level` (`hooks/base.rs`) | Fires after the entity is in the level's lists; cancellable, unlike the method. |
| `onRemovedFromLevel()` | Partial | `entity_leave_level` (`hooks/base.rs`) | Fires after removal and only listens. |
| `revive()` | Missing | — | No entity revive API. |
| `isMultipartEntity()` | Missing | — | The dragon's parts are internal to the engine. |
| `getParts()` | Missing | — | Same; `EntityRef` names dragon parts in some reads but does not expose the array. |
| `getFluidTypeHeight(type)` | Missing | — | Fluid properties are per-fluid data; no per-entity height. |
| `getMaxHeightFluidType()` | Missing | — | Same. |
| `isInFluidType(state)` | Partial | `host.entity_is_in_water`, `entity_is_in_lava` | Only water and lava answered; no arbitrary fluid. |
| `isInFluidType(type)` | Partial | `host.entity_is_in_water`, `entity_is_in_lava` | Same. |
| `isInFluidType(predicate)` | Partial | `host.entity_is_in_water`, `entity_is_in_lava` | No predicate form. |
| `isInFluidType()` | Partial | `host.entity_is_in_water`, `entity_is_in_lava` | "Any fluid" is water-or-lava only. |
| `getFirstEyeInFluidType()` | Missing | — | Eye position per fluid is engine-owned. |
| `isEyeInFluidType(type)` | Missing | — | Same. |
| `canStartSwimming()` | Missing | — | Swimming start is engine-owned. |
| `getFluidMotionScale(type)` | Missing | — | No per-entity fluid scale. |
| `isPushedByFluid(type)` | Partial | `FluidTypeProperties.can_push_entity` | Per-fluid data, not a per-entity answer. |
| `canSwimInFluidType(type)` | Partial | `FluidTypeProperties.can_swim` | Per-fluid data. |
| `canFluidExtinguish(type)` | Partial | `FluidTypeProperties.can_extinguish` | Per-fluid data. |
| `getFluidFallDistanceModifier(type)` | Partial | `FluidTypeProperties.fall_distance_modifier` | Per-fluid data. |
| `canHydrateInFluidType(type)` | Partial | `FluidTypeProperties.can_hydrate` | Per-fluid data. |
| `getSoundFromFluidType(type, action)` | Missing | — | No fluid sound-action table. |
| `sendPairingData(player, consumer)` | Missing | — | No pairing-payload API. |
| `copyAttachmentsFrom(entity, wasDeath)` | Partial | attachments survive dimension change; player respawn keeps `copy_on_death` entries; mob conversion copies every entry with a codec; `player_clone` (`hooks/base.rs`) | Copy rules are the attachment store's; there is no per-entity hook at the copy. |
| `getBlockBounciness(pos, state)` | Partial | `BlockPart::bounce_on` (`hooks/block_landing.rs`) | Edits post-bounce movement instead of returning a bounciness factor. |

## `ILivingEntityExtension`

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `canSwimInFluidType(type)` | Partial | `FluidTypeProperties.can_swim` | Per-fluid data, not per living entity. |
| `jumpInFluid(type)` | Missing | — | Fluid jump behaviour is engine-owned. |
| `sinkInFluid(type)` | Missing | — | Same. |
| `canDrownInFluidType(type)` | Partial | `living_drown` (`hooks/base.rs`), `FluidTypeProperties.can_drown` | The global drown event can cancel or change damage; fluid data decides whether it applies. |
| `moveInFluid(type, movement, fallDistance)` | Missing | — | Movement in fluid is engine-owned; no per-entity method. |
| `onDamageTaken(container)` | Partial | `living_damage_post` (`hooks/living_damage_post.rs`), `living_incoming_damage` (`hooks/base.rs`) | Global server hooks: the incoming container before the hit, and a post-damage notice only once health dropped, not a per-entity callback for every hurt. |

## `IBlockEntityExtension`

Block entity parts are attached with `r.block_entity_part`; `BlockEntityPart` extends `BlockPart` and carries the
block-entity hooks (`crates/horizon-mod-vocab/src/hooks/` with `scope = "block_entity"`). Saved data and attachments
are reachable through `BlockEntityRef`.

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `onDataPacket(connection, input)` | Partial | `BlockEntityPart::block_entity_update_tag` (`hooks/base.rs`) | The server half (`getUpdateTag`) is settable; the client's merge of the received tag is engine-side. |
| `handleUpdateTag(input)` | Partial | `BlockEntityPart::block_entity_update_tag` (`hooks/base.rs`) | Same: the data is covered, the client callback is not. |
| `getPersistentData()` | Covered | `AttachmentStore`, `AttachmentHolder::BlockEntity` | Typed saved/synced attachments replace the writable compound. |
| `onChunkUnloaded()` | Covered | `BlockEntityPart::on_chunk_unloaded` (`hooks/on_chunk_unloaded.rs`) | Once per mod block entity of an unloading chunk, on each side holding it, before it is dropped; not when the world closes. |
| `onLoad()` | Partial | `BlockEntityPart::block_entity` (`hooks/base.rs`) | Initial saved data is set at creation, before the saved NBT is applied; no after-load notice. |
| `requestModelDataUpdate()` | Missing | — | No model-data invalidation API. |
| `getModelData()` | Missing | — | No `ModelData`; drawing uses `block_entity_submit` and the model path. |
| `invalidateCapabilities()` | Partial | `BlockCapabilityCache::invalidate` (`crates/horizon-mod-vocab/src/item/capability_cache.rs`) | Providers are queried per lookup; a `BlockCapabilityCache` keeps one and checks it by polling. When a provider's answer changes with nothing the cache compares, the cache's owner calls `invalidate`; there is no broadcast to every cache. |
| `applyStructureRotation(mirror, rotation)` | Missing | — | Structure rotation is engine-owned; no callback. |

## `IFluidExtension`

Fluid type facts are one record, `FluidTypeProperties` (`crates/horizon-mod-vocab/src/block/fluid_states.rs`), read with
`host.fluid_states().type_properties(key)`. They are static per fluid, not per position or block, and they are
derived today: the record describes water's and lava's families and gives every other fluid inert values
(`crates/horizon-client/src/game/modding/native_block_states.rs`). A mod's own fluid declares a sprite and tint,
not its type properties, so "partial" below means the vanilla answer is readable but a mod fluid cannot set it.

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `getExplosionResistance(state, level, pos, explosion)` | Missing | — | No fluid explosion resistance. |
| `getFluidType()` | Partial | `host.fluid_states().type_properties` | Reads water's and lava's type record; a mod's own fluid cannot declare one and reads as inert. |
| `canConvertToSource(state, level, pos)` | Partial | `create_fluid_source` (`hooks/create_fluid_source.rs`) | Global `CreateFluidSourceEvent`; not per-fluid and not a Part. |
| `supportsBoating(state, boat)` | Partial | `FluidTypeProperties.supports_boating` | Static, and only a water-family fluid supports boating; a mod fluid cannot opt in. |
| `getBlockPathType(state, level, pos, mob, canFloat)` | Partial | `BlockPart::is_pathfindable` (`hooks/is_pathfindable.rs`) | Block-level path computation type only; no fluid `PathType`. |
| `getAdjacentBlockPathType(state, level, pos, mob, pathType)` | Missing | — | No adjacent fluid path type. |
| `canHydrate(state, level, pos, farmland)` | Partial | `FluidTypeProperties.can_hydrate` | Static, and only a water-family fluid hydrates; a mod fluid cannot opt in. |
| `canExtinguish(state, level, pos)` | Partial | `FluidTypeProperties.can_extinguish` | Static, and only a water-family fluid extinguishes; a mod fluid cannot opt in. |
| `appendHoverText(stack, context, display, lines, flag)` | Missing | — | `FluidStack` has no tooltip hook. |

## `IMenuTypeExtension`

Menu types are content data (`MenuType` in `crates/horizon-mod-package/src/content.rs`, files under
`data/<id>/horizon/menu/`), and the game's own menu engine opens them (`menu::Kind::Mod`). A `MenuPart` can answer
slot rules and whether a player's block menu stays open (`menu_still_valid`, vanilla's `stillValid`), not
construct a menu.

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `create(IContainerFactory)` (static) | Partial | content `MenuType` declaration | Declares slot count, data slots and take-only slots, but cannot supply a factory that builds the client menu from server-sent data. |
| `create(windowId, inventory, buffer)` | Partial | menu engine + `MenuPart::menu_slot_rules` | The engine builds the menu from the declaration; a mod cannot run code at construction. |

## What the follow-up issues cover

[#1038](https://github.com/arismoko/horizon/issues/1038) (blocks) and
[#1039](https://github.com/arismoko/horizon/issues/1039) (items) were written from the earlier audit. Some of
their rows have landed or become partial since and should be narrowed when they are filed:

- **Changed on the block side:** `shouldCheckWeakPower` and `should_redstone_wire_connect_to` (the issue says
  reuse them) are Parts; `getFlammability` / `getFireSpreadSpeed` are now `fire_odds` (with position and face), with `is_fire_source`, `ignited_by_lava` and
  `on_caught_fire` for the other fire questions; `canSustainPlant` is `can_sustain_plant` (the soil) before `may_place_on` (the
  plant); `canHarvestBlock` is `can_harvest_block` before `player_harvest_check`; `isLadder` is `is_ladder`;
  `onTreeGrow` is `on_tree_grow`; `getToolModifiedState` is `tool_modified_state` before
  `block_tool_modification`; `onDestroyedByPlayer` is `on_destroyed_by_player`.
- **Changed on the item side:** `getEquipmentSlot` is `ItemResource::equippable_slot`; `getCraftingRemainder`
  has `ItemPart::crafting_remainder` plus content data and `host.item_crafting_remainder`; `canEquip` is `ItemPart::can_equip`; `getXpRepairRatio` is `ItemPart::xp_repair_ratio`.
- **Block group rows that still need their own answer:** `getExpDrop`, `onBlockExploded`,
  `getWeakChanges`, `onNeighborChange`, `getBounceRestitution`, `isSlimeBlock`, `isStickyBlock`,
  `canStickTo` (all Partial or Missing above).
- **Item group rows that still need their own answer:** `onEntityItemUpdate`, `canPerformAction` (both
  Partial or Missing above).
- **Missing but not in either group:** the remaining block, entity, block-entity and fluid rows marked Missing
  above; they have no follow-up issue yet. The largest are the entity fluid queries, block entity model data and
  fluid tooltips.

### Resource-backed block-part checks

With `HORIZON_TEST_RESOURCES` pointing to prepared resources, run the tool and player-removal
fixtures together (including the ice-denial regression):

```sh
for suite in block_part_tool_tests player_destroy_tests; do
    cargo test -p horizon-client --lib "$suite" -- --ignored
done
```

These checks require prepared resources; they are not ordinary resource-free unit tests.

## IClientBlockExtensions: mining hit sound

| Method | Verdict | Horizon route |
| --- | --- | --- |
| `IClientBlockExtensions.playHitSound` | **Covered** | Client `BlockPart::play_hit_sound`, with state, position, face and `ClientHost`. Returning `Flow::Cancel` handles the hit and suppresses the default sound. Both local mining ticks and `ClientLevel.addBreakingBlockEffects` use the same sound helper. Sculk growth is outside this hook. |

## Client chunk events

This section is not an extension interface. It records where NeoForge's client-side `ChunkEvent` posts land in
Horizon, as #3425 asks.

| NeoForge | Verdict | Horizon route |
| --- | --- | --- |
| `ChunkEvent.Load` on the client (`isNewChunk` always false) | **Covered** | `client_chunk_load(dimension, chunk)`, once per adopted chunk packet after the column and its block entities are in the client level; a re-sent column is told again. |
| `ChunkEvent.Unload` on the client | **Covered, wider** | `client_chunk_unload(dimension, chunk)`, before the column's data is freed, on every route a column leaves by: the forget packet (NeoForge's only route), the view centre moving, the view radius shrinking and the client level going. |

Both are client-only, so the server's `chunk_load` and `chunk_unload` listeners are not called on a client. An
integrated world's own chunks are told only through the server hooks.

## IShearable: shearing entities

| Method | Verdict | Horizon route |
| --- | --- | --- |
| `IShearable.isShearable(player, item, level, pos)` | **Covered** | `EntityPart::is_shearable(host, entity, player, shears)`, server-side, asked by a player's shears and a dispenser's before `player_shear_entity` / `block_shear_entity`; the first part to answer wins, `None` leaves it to the species. A `false` from a part ends a player's click with nothing done. |
| `IShearable.onSheared(player, item, level, pos)` | **Covered** | `EntityPart::on_sheared(host, entity, player, shears)`, called on the part that said yes once the cancel hook passed; it returns the drops. |
| `IShearable.spawnShearedDrop(level, pos, drop)` | **Covered, engine-side** | The game spawns each returned drop one block above the entity with the sheep's random nudge, posts the shear game event and damages the shears. Not overridable per part. |

Vanilla's shearable species (sheep, mooshroom, snow golem, bogged, copper golem, sulfur cube) keep their own
shearing when no part answers. Entity parts are native-only, so wasm mods do not have these methods yet.

## IFallableExtension: falling tick

| Method | Verdict | Horizon route |
| --- | --- | --- |
| `IFallableExtension.fallingTick(level, currentPosition, entity)` | **Partial** | Server `BlockPart::falling_tick` (`hooks/falling_tick.rs`) on the falling block's state, with its block position and entity handle and a `&mut dyn ServerHost`, for each tick that ends with it still in the air; not the landing, drop or removal tick. It is delivered after the ordered pass that ticked the falling block, so a write lands on the stored entity. NeoForge also calls it on the client; Horizon calls it only on the server. |
