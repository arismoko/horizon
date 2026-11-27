use super::*;

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// The server is starting a world save cycle, before any writes.
    pub(in crate::game) fn native_world_save_start(&mut self, wait: bool) {
        self.fire_world_save_start(wait);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_native_world_save_start(&mut self, wait: bool) {
        self.native_world_save_start(wait);
    }

    /// `WorldlyContainer.canTakeItemThroughFace` for a block entity's subscribed part: whether
    /// `stack` in `slot` may leave through `face`, replacing the declared output slots.
    pub(in crate::game) fn native_can_take_item_through_face(
        &self,
        entity: &mod_api::ModBlockEntity,
        pos: BlockPos,
        state: mod_api::BlockState,
        slot: usize,
        stack: &mod_api::ItemStack,
        face: Direction,
        allowed: &mut Option<bool>,
    ) {
        self.fire_can_take_item_through_face(entity, pos, state, slot, stack, face, allowed);
    }
    /// A player took an anvil's result at `pos` and the levels, inputs and anvil wear are done (`AnvilCraftEvent.Post`); a broken anvil is already air. `inputs` borrows the input stacks and result from before the take. Not called when `native_anvil_craft_pre` cancelled.
    pub(in crate::game) fn native_anvil_craft_post(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        inputs: &mod_api::item::menu::AnvilCraftInputs<'_>,
    ) {
        self.fire_anvil_craft_post(player, name, pos, inputs);
    }
    /// A player took an anvil's result at `pos` (`AnvilCraftEvent.Pre`), before the levels are charged, the inputs used up and the anvil worn. `inputs` borrows the input stacks and result from before the take. Set `cancel` to return early before charging levels, consuming inputs, damaging the anvil or firing `anvil_craft_post`, without undoing the click's item move (matching NeoForge `AnvilMenu.onTake`).
    pub(in crate::game) fn native_anvil_craft_pre(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        inputs: &mod_api::item::menu::AnvilCraftInputs<'_>,
        cancel: &mut bool,
    ) {
        self.fire_anvil_craft_pre(player, name, pos, inputs, cancel);
    }
    /// A player takes an item from an armor stand or puts one on it
    /// (`ArmorStand.interactAt`, after the stand's disabled-slot checks and
    /// `swapItem`'s take/put locks, before anything moves): `stand_id` is the
    /// stand's `Entity.getId` and `slot` the one the click reached
    /// (`mainhand`, `offhand`, `feet`, `legs`, `chest` or `head`). Set
    /// `cancel` and nothing changes: neither the stand nor the player's hand,
    /// and the click is spent. The player's state is swapped in for the call;
    /// `player` is the UUID the helpers take. Not told of a click that changes
    /// nothing (an empty hand on an empty slot). Not covered, other ways a
    /// stand's equipment moves: breaking or blasting the stand (its items
    /// drop), a dispenser putting armour on one, a mod's own changes.
    /// Server-side, for every player of the world.
    pub(in crate::game) fn native_armor_stand_manipulate(
        &mut self,
        player: u128,
        name: &str,
        stand_id: i32,
        slot: &str,
        cancel: &mut bool,
    ) {
        self.fire_armor_stand_manipulate(player, name, stand_id, slot, cancel);
    }
    /// NeoForge's `ArrowLooseEvent`: the player releases a bow `stack` with
    /// `has_ammo` and `charge` ticks held. Change `charge` or set `canceled`
    /// to prevent the shot; negative charge also prevents it. Unlike
    /// `native_living_entity_use_item_stop`, this does not skip the whole item
    /// release; `native_entity_shoot_bow` reports a spawned arrow afterward
    /// with read-only force. Fires on the Game running the bow release, including
    /// LAN guests; a remote client's own release path is not covered. Crossbows
    /// are not covered; `native_arrow_nock` is the bow's start.
    pub(crate) fn native_arrow_loose(
        &mut self,
        stack: &ItemStack,
        has_ammo: bool,
        charge: &mut i32,
        canceled: &mut bool,
    ) {
        self.fire_arrow_loose(stack, has_ammo, charge, canceled);
    }
    /// A player attacks the entity `entity` (`Entity.getId`) of type `kind`
    /// (`minecraft:item_frame`, `minecraft:cow`, `minecraft:player`) standing
    /// at `pos`: `ServerGamePacketListenerImpl.handleInteract`'s attack, once
    /// in reach, where `Player.attack` starts. The swing has not begun, so the
    /// attacking player's attack strength is still the charge the blow uses
    /// (vanilla reads it at the start of `Player.attack`; the swing resets it
    /// afterwards). Set `canceled` and the blow does nothing (no damage, no
    /// knockback, the frame keeps its item). Server-side, for every player of
    /// the world (`native_acting_player` says who).
    pub(crate) fn native_attack_entity(
        &mut self,
        entity: i32,
        kind: &str,
        pos: [f64; 3],
        canceled: &mut bool,
    ) {
        self.fire_attack_entity(entity, kind, pos, canceled);
    }
    /// A beacon at `pos` activated or deactivated after its beam was checked.
    /// `active` says whether it now has a complete pyramid. Server-side.
    pub(in crate::game) fn native_beacon_state_change(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        active: bool,
    ) {
        self.fire_beacon_state_change(dimension, pos, active);
    }
    /// A server guest's block-breaking animation changed to `progress` at
    /// `pos` (0 through 9).
    pub(crate) fn native_block_break_progress_update(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        progress: i32,
    ) {
        self.fire_block_break_progress_update(player, name, pos, progress);
    }
    /// A block finished cooking `item` into `result` at `pos`.
    pub(crate) fn native_block_cook(&mut self, pos: [i32; 3], item: &str, result: &str) {
        self.fire_block_cook(pos, item, result);
    }
    /// A server player stopped digging `pos` before the block broke, after
    /// the server cleared the dig and sent the block back to the client.
    /// The player's state is not swapped in.
    pub(crate) fn native_block_damage_abort(&mut self, player: u128, name: &str, pos: [i32; 3]) {
        self.fire_block_damage_abort(player, name, pos);
    }
    /// Block `block`'s registered item slots (`WorldlyContainer`), read from
    /// its per-block declaration when the ticking table is built.
    /// Registration already checks the saved slot count. The table builder leaves
    /// out slot references past that count.
    /// Its menu must be one of the supported menus:
    /// `containers::MOD_MENUS` or a content mod's menu type
    /// (`data/<id>/sunrise/menu/`) with as many
    /// slots, opens when the block is used (its template must open a menu, as
    /// a chest or a furnace does); another is left out. A mod menu type shows
    /// on Sunrise clients through the mod's registered screen factory.
    pub(in crate::game) fn native_block_entity_container(
        &self,
        block: &str,
        container: &mut Option<ModContainer>,
    ) {
        *container = mod_api::__private::parts_installation::block_container(block).cloned();
    }
    /// Block `block`'s registered energy buffer (`Capabilities.Energy.BLOCK`):
    /// its capacity, the most one insert and one extract moves, and the faces
    /// a fill may come through and a drain may leave by.
    /// Read from its per-block declaration when the ticking
    /// table is built; a capacity under 1 leaves the buffer out, and the
    /// limits are clamped to 0..=capacity. The amount is kept in the entity's
    /// saved compound, as the int `energy`, and the energy helpers below read
    /// and write it.
    pub(in crate::game) fn native_block_entity_energy(
        &self,
        block: &str,
        energy: &mut Option<ModEnergyBuffer>,
    ) {
        *energy = mod_api::__private::parts_installation::block_energy(block);
    }
    /// Block `block`'s registered fluid tanks: each tank's capacity in
    /// millibuckets, the tanks each face reaches, and the tanks a fill may
    /// reach and a drain may take. Read from its per-block declaration when the
    /// ticking table is built; at most 16 tanks, capacities clamped to
    /// 1..=1 000 000, and tank numbers past the count are left out. The tanks'
    /// contents are kept in the entity's saved compound, under `fluids`, and
    /// the fluid helpers below read and write them.
    pub(in crate::game) fn native_block_entity_fluids(
        &self,
        block: &str,
        fluids: &mut Option<ModFluidTanks>,
    ) {
        *fluids = mod_api::__private::parts_installation::block_fluids(block);
    }
    /// A sculk sensor or shrieker received a vibration for `event` at `pos`.
    /// Set `cancel` to suppress its response. Server-side.
    pub(in crate::game) fn native_block_receive_game(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        block: &str,
        event: &str,
        cancel: &mut bool,
    ) {
        self.fire_block_receive_game(dimension, pos, block, event, cancel);
    }
    /// A player breaks the block `state` at `pos`, once the game allowed it
    /// (the held item can break blocks, the debug stick didn't take the
    /// click): where `ServerPlayerGameMode.destroyBlock` starts. Set
    /// `canceled` to keep the block: nothing is destroyed or dropped, and a
    /// guest sees it back. Server-side, for every player of the world (the
    /// helpers read the acting one).
    pub fn native_break_block(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        canceled: &mut bool,
        exp_to_drop: &mut Option<u32>,
    ) {
        self.prepare_part_break_experience(pos, state, exp_to_drop);
        let initial = *exp_to_drop;
        self.fire_break_block(pos, state, canceled, exp_to_drop);
        *exp_to_drop = exp_to_drop.or(initial);
    }
    /// A brewing stand at `pos` consumed `fuel` and gained brewing fuel. Server-side.
    pub(crate) fn native_brewing_fuel(&mut self, pos: [i32; 3], fuel: &str) {
        self.fire_brewing_fuel(pos, fuel);
    }
    /// A brewing stand at `pos` began brewing `ingredient`, after fuel and the timer were set. Server-side.
    pub(crate) fn native_brewing_start(&mut self, pos: [i32; 3], ingredient: &str) {
        self.fire_brewing_start(pos, ingredient);
    }
    /// A server player empties a bucket at `pos` (`fluid`: `minecraft:water`,
    /// `minecraft:lava`, `minecraft:powder_snow`; a fish or axolotl bucket
    /// empties `minecraft:water`): where `BucketItem.use` has found the block
    /// the fluid goes into and the player may build there, right before it is
    /// placed. Set `cancel` and nothing is poured and the bucket stays full.
    /// The player's state is swapped in, so `native_tell` answers them. Only
    /// for guests of a server. `native_fill_bucket` hears this click too (and
    /// the filling one) without the player and the fluid. Not heard: a
    /// dispenser, a cauldron, and a fluid a block places itself.
    pub(crate) fn native_bucket_empty(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        fluid: &str,
        cancel: &mut bool,
    ) {
        self.fire_bucket_empty(player, name, pos, fluid, cancel);
    }
    /// An item was placed on a campfire at `pos` and began cooking.
    pub(crate) fn native_campfire_start(&mut self, pos: [i32; 3], item: &str) {
        self.fire_campfire_start(pos, item);
    }
    /// Chunk `chunk` (`[x, z]`, in chunks) of `dimension` came into the world
    /// (NeoForge's `ChunkEvent.Load`): `new` when it was just generated, false
    /// when read from the save. A flat world's chunk that leaves before it's
    /// first lit isn't saved, so it's generated, and new, again; one generated
    /// as the players left is new when it next comes, unless the server
    /// restarts first, or the level closes for being empty, which drops its
    /// memory of what never saved. Its blocks and block entities are there,
    /// but its saved entities join afterwards and its light may not be ready:
    /// change the world from a later tick. The chunks around the spawn come
    /// before `WorldOpenReady`. Not told of a chunk a client receives.
    /// Server-side.
    pub(in crate::game) fn native_chunk_load(
        &mut self,
        dimension: &str,
        chunk: [i32; 2],
        new: bool,
    ) {
        self.fire_chunk_load(dimension, chunk, new);
    }
    /// A compostable item was added to the composter at `pos`.
    pub(crate) fn native_compost_item(&mut self, pos: [i32; 3], item: &str) {
        self.fire_compost_item(pos, item);
    }
    /// A creative-mode player sets a slot of their inventory from the
    /// creative screen (`ServerGamePacketListenerImpl.handleSetCreativeModeSlot`):
    /// `slot` is the inventory menu's slot (5 to 8 armour, 9 to 35 the
    /// inventory, 36 to 44 the hotbar, 45 the off hand; below 0 is a stack
    /// thrown out of the screen, which is dropped) and `item` (`minecraft:air`
    /// for an emptied slot) and `count` what is put there. Heard once the
    /// packet passed vanilla's checks (a creative player, a real slot, a valid
    /// stack) and before the slot changes or the stack drops. Set `cancel`
    /// and the packet is ignored. The player's state is swapped in for the
    /// call, as for a command executor, so `native_acting_player`,
    /// `native_tell` and the item helpers reach them. Not heard: stacks a
    /// creative player gets other ways (middle-click pick block, `/give`).
    /// Server-side, for every player of the world.
    pub(crate) fn native_creative_slot(
        &mut self,
        player: u128,
        name: &str,
        slot: i32,
        item: &str,
        count: i32,
        cancel: &mut bool,
    ) {
        self.fire_creative_slot(player, name, slot, item, count, cancel);
    }
    /// A player ignited a creeper with an item.
    pub(in crate::game) fn native_creeper_ignite(&mut self, entity: u32, player: u128, item: &str) {
        self.fire_creeper_ignite(entity, player, item);
    }
    /// Lightning charged a creeper.
    pub(in crate::game) fn native_creeper_power(&mut self, entity: u32, lightning: [f64; 3]) {
        self.fire_creeper_power(entity, lightning);
    }
    /// A server player applied an enchantment at `pos`; `item` is the finished item and `levels` the levels charged. The player's state is not swapped in.
    pub(in crate::game) fn native_enchant_item(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        item: &ItemStack,
        levels: i32,
    ) {
        self.fire_enchant_item(player, name, pos, item, levels);
    }
    /// A server player's or mob's ender pearl is about to teleport its `owner`
    /// (`EntityTeleportEvent.EnderPearl`, `ThrownEnderpearl.onHit`). `owner` is
    /// the live entity handle, `pearl` its projectile id, `from` the owner's feet
    /// and `target` the pearl's previous position. A mod may move the target;
    /// non-finite changes keep the original spot. Canceling leaves the owner
    /// in place with no teleport sound, damage or endermite roll; the pearl is
    /// used up. Player owners keep their acting-player context and mutable fall
    /// `damage` (5; nonpositive is none). Mob owners take no fall damage and
    /// make no endermite draw; their `damage` starts at zero and is player-only.
    /// An owner moved, removed, killed or put to sleep by a listener is not moved
    /// again using the old decision. The impact hook still hears the hit.
    pub(in crate::game) fn native_ender_pearl_teleport(
        &mut self,
        call: mod_api::__private::EnderPearlTeleportCall<'_>,
    ) {
        self.fire_ender_pearl_teleport(call);
    }
    /// Entities in `chunk` left the active server world and were queued for
    /// saving. `count` is the number of entity records.
    pub(in crate::game) fn native_entities_unload(
        &mut self,
        dimension: &str,
        chunk: [i32; 2],
        count: usize,
    ) {
        self.fire_entities_unload(dimension, chunk, count);
    }
    /// A mob broke the door at `pos`.
    pub(in crate::game) fn native_entity_break_door(
        &mut self,
        entity: u32,
        kind: &str,
        pos: [i32; 3],
    ) {
        self.fire_entity_break_door(entity, kind, pos);
    }
    /// Two mobs completed a breeding action. `child` is absent when vanilla
    /// lays an egg instead of creating a baby.
    pub(in crate::game) fn native_entity_breed(
        &mut self,
        parent: u32,
        partner: u32,
        child: Option<u32>,
    ) {
        self.fire_entity_breed(parent, partner, child);
    }
    /// A mob changed a block at `pos`, after the new state was applied.
    pub(in crate::game) fn native_entity_change_block(
        &mut self,
        entity: u32,
        kind: &str,
        pos: [i32; 3],
        from: &str,
        to: &str,
    ) {
        self.fire_entity_change_block(entity, kind, pos, from, to);
    }
    /// A mob is about to drop `count` of `item` at `pos` (Bukkit's
    /// `EntityDropItemEvent`).
    pub(in crate::game) fn native_entity_drop_item(
        &mut self,
        entity: u32,
        item: &str,
        count: i32,
        pos: [f64; 3],
    ) {
        self.fire_entity_drop_item(entity, item, count, pos);
    }
    /// A bee entered the hive at `pos` and was stored there.
    pub(crate) fn native_entity_enter_block(
        &mut self,
        entity: u32,
        kind: &str,
        pos: [i32; 3],
        block: &str,
    ) {
        self.fire_entity_enter_block(entity, kind, pos, block);
    }
    /// A mob entered love mode after player interaction. `player` is the
    /// feeder when known.
    pub(in crate::game) fn native_entity_enter_love_mode(
        &mut self,
        entity: u32,
        kind: &str,
        player: Option<u128>,
    ) {
        self.fire_entity_enter_love_mode(entity, kind, player);
    }
    /// An entity joins the server level `dimension` (NeoForge's
    /// `EntityJoinLevelEvent`): vanilla's `PersistentEntitySectionManager.addEntity`.
    /// `loaded_from_disk` is true for an entity coming back with its chunk,
    /// false for one added fresh (`addFreshEntity`: a drop, an orb, `/summon`,
    /// an arrival through a portal). `entity` is its ID and `kind` its type
    /// (`minecraft:item`). It is already in the level's lists, so helpers that
    /// take an entity ID find it. Set `canceled` to keep it out: it is taken
    /// back out as if never added, and a `/summon` of it fails. Tells every
    /// kind in the game's entity lists (items, orbs, mobs, boats, minecarts,
    /// armor stands, primed TNT, falling blocks, projectiles, rockets,
    /// tridents, end crystals, the dragon, cushions, paintings, item frames),
    /// once each, in the order they were created; check `kind`. A drop, an
    /// orb, a mob, a `/summon` and a chunk's entities are told at once, any
    /// other spawn between the tick's phases (it is checked after the blocks,
    /// the vehicles, the projectiles, the items and the mobs, and at the
    /// tick's end). A mod that cancels one of the others sees it removed
    /// before it ticks again. Not told: the entities Sunrise keeps without
    /// simulating (displays, markers, interactions). Server-side.
    pub(crate) fn native_entity_join_level(
        &mut self,
        dimension: &str,
        kind: &str,
        pos: [f64; 3],
        loaded_from_disk: bool,
        subject: mod_api::EntityRef,
        canceled: &mut bool,
    ) {
        self.fire_entity_join_level(dimension, kind, pos, loaded_from_disk, subject, canceled);
    }
    /// A server entity finished its portal wait and is beginning to leave
    /// `dimension` through `pos`. The destination follows the portal rules.
    pub(in crate::game) fn native_entity_portal(
        &mut self,
        dimension: &str,
        entity: u32,
        pos: [i32; 3],
    ) {
        self.fire_entity_portal(dimension, entity, pos);
    }
    /// A guest player's synchronized pose changed from `from` to `to`.
    pub(in crate::game) fn native_entity_pose_change(
        &mut self,
        player: u128,
        name: &str,
        from: &str,
        to: &str,
    ) {
        self.fire_entity_pose_change(player, name, from, to);
    }
    /// A mob became tamed after its owner and persistent state were set.
    pub(in crate::game) fn native_entity_tame(
        &mut self,
        entity: u32,
        kind: &str,
        owner: Option<u128>,
    ) {
        self.fire_entity_tame(entity, kind, owner);
    }
    /// A guest's fall flying state changed on the server. The change has
    /// already been applied.
    pub(crate) fn native_entity_toggle_glide(&mut self, player: u128, name: &str, gliding: bool) {
        self.fire_entity_toggle_glide(player, name, gliding);
    }
    /// An experience bottle broke and its experience was awarded.
    pub(crate) fn native_exp_bottle(&mut self, pos: [f64; 3], experience: u32) {
        self.fire_exp_bottle(pos, experience);
    }
    /// Whether the experience orb `into` takes the orb `from` in (vanilla's
    /// `ExperienceOrb.canMerge`): the same value, and ids a multiple of 40 apart. Asked
    /// for each pair whose boxes meet, when an orb spawns (`tryMergeToExisting`: `from` is
    /// 0, the new orb isn't an entity yet, and only `from_value` is known) and every 20
    /// ticks as an orb scans for neighbours (`scanForMerges`). `can_merge` holds vanilla's
    /// answer; change it either way. A merged orb counts as `count` orbs of `into`'s
    /// value, so forcing orbs of different values together drops points unless the mod
    /// raises `into`'s value with `set_experience_orb` in `native_experience_orb_merge`.
    /// Server-side.
    pub(crate) fn native_experience_orb_can_merge(
        &mut self,
        dimension: &str,
        into: u32,
        from: u32,
        into_value: i32,
        from_value: i32,
        can_merge: &mut bool,
    ) {
        self.fire_experience_orb_can_merge(
            dimension, into, from, into_value, from_value, can_merge,
        );
    }
    /// The experience orb merge hook's entry point: its arguments are declared on the hook.
    pub(crate) fn native_experience_orb_merge(
        &mut self,
        call: mod_api::__private::ExperienceOrbMergeCall<'_>,
    ) {
        self.fire_experience_orb_merge(call);
    }
    /// A player fills (`stack` an empty bucket) or empties a bucket at `pos`:
    /// `BucketItem.use`, where it asks `Level.mayInteract`. Set `canceled`
    /// and the bucket does nothing. Server-side, for every player of the
    /// world (`native_acting_player` says who).
    pub(in crate::game) fn native_fill_bucket(
        &mut self,
        stack: &ItemStack,
        pos: [i32; 3],
        canceled: &mut bool,
    ) {
        self.fire_fill_bucket(stack, pos, canceled);
    }
    /// A firework rocket completed its explosion effects.
    pub(crate) fn native_firework_explode(&mut self, entity: u32, pos: [f64; 3], bursts: usize) {
        self.fire_firework_explode(entity, pos, bursts);
    }
    /// A furnace at `pos` consumed fuel and began or resumed burning it.
    /// `fuel` is the item consumed from its fuel slot. Server-side.
    pub(crate) fn native_furnace_burn(&mut self, pos: [i32; 3], fuel: &str) {
        self.fire_furnace_burn(pos, fuel);
    }
    /// A furnace finished smelting `item` into `result` at `pos`. Server-side.
    pub(crate) fn native_furnace_smelt(&mut self, pos: [i32; 3], item: &str, result: &str) {
        self.fire_furnace_smelt(pos, item, result);
    }
    /// A furnace at `pos` began cooking `item` (its cooking timer advanced
    /// from zero). Server-side.
    pub(crate) fn native_furnace_start_smelt(&mut self, pos: [i32; 3], item: &str) {
        self.fire_furnace_start_smelt(pos, item);
    }
    /// A server player places a hanging entity: an item frame
    /// (`minecraft:item_frame`, `minecraft:glow_item_frame`), a painting
    /// (`minecraft:painting`) or a leash knot (`minecraft:leash_knot`, when a
    /// lead is tied to a fence that has none yet), `kind` its entity type and
    /// `pos` the block it takes (the fence's for a knot). Where
    /// `HangingEntityItem.useOn` and `LeadItem.useOn` create it, once it is
    /// known to fit and before anything is placed or used up. Set `cancel`
    /// and nothing is placed: the item stays in the player's hand. The
    /// player's state is swapped in, so `native_tell` answers them. Only for
    /// guests of a server; `/summon`, a dispenser and the game's own player
    /// are not heard, nor is a lead tied to a mob or a boat.
    pub(crate) fn native_hanging_place(
        &mut self,
        player: u128,
        name: &str,
        kind: &str,
        pos: [i32; 3],
        cancel: &mut bool,
    ) {
        self.fire_hanging_place(player, name, kind, pos, cancel);
    }
    /// A hopper selected a container while searching for items to push or
    /// pull. Exactly one of `container` and `entity` identifies the target.
    pub(in crate::game) fn native_hopper_inventory_search(
        &mut self,
        hopper: [i32; 3],
        container: Option<[i32; 3]>,
        entity: Option<u32>,
    ) {
        self.fire_hopper_inventory_search(hopper, container, entity);
    }
    /// A player's hit landed on the mob `entity` of type `kind`
    /// (`minecraft:zombie`), holding `stack`: where `Item.hurtEnemy` is called,
    /// before the post-attack enchantments and the weapon's wear. It only
    /// hears. Another player as the target isn't told yet. Server-side, for
    /// every player of the world (the helpers read the acting one).
    pub(in crate::game) fn native_hurt_enemy(
        &mut self,
        stack: mod_api::ItemStackView<'_>,
        entity: u32,
        kind: &str,
    ) {
        self.fire_hurt_enemy(stack, entity, kind);
    }
    /// A server tick of `stack`, the stack a player carries in `slot`: where
    /// `Inventory.tick` calls `Item.inventoryTick`, for every stack, each tick,
    /// `/tick freeze` included (it stops the world, not the player). `slot` is
    /// as `Inventory` saves it (0 to 35; the armour 100 feet to 103 head; -106
    /// the off hand), and `selected` says it is the held slot.
    /// `ItemPlace::PlayerSlot { player, slot }` reads and changes the stack,
    /// which stays in its slot: an emptied one empties the slot, and the
    /// player's client gets the change. Only the `ItemPart`s attached to
    /// `stack`'s item hear it. Server-side, for every living player of the world
    /// (the helpers read the acting one).
    pub(in crate::game) fn native_inventory_tick(
        &mut self,
        item: mod_api::ItemKey,
        place: &mod_api::ItemPlace,
        slot: i32,
        selected: bool,
    ) {
        self.fire_inventory_tick(item, place, slot, selected);
    }
    /// A server player or fox that ate chorus fruit (or another item with the
    /// `teleport_randomly` consume effect) is about to try a random spot
    /// (NeoForge's `EntityTeleportEvent.ItemConsumption`, vanilla's
    /// `TeleportRandomlyConsumeEffect.apply`): once per attempt (up to 16),
    /// after they stopped riding. `from` is their feet, `target` the spot
    /// drawn; a mod may move it (its y kept inside the level's height, a
    /// value that is not a number ignored), and the game still looks down
    /// from it for ground it may land on. Canceling ends the effect: no teleport and no
    /// more attempts. `consumed` is the stack captured before use, including its
    /// count and components. The player's state is swapped in
    /// (`native_acting_player`); a fox's full-host callback runs at its inline
    /// consumption boundary, before living AI and movement.
    pub(in crate::game) fn native_item_consumption_teleport(
        &mut self,
        consumer: mod_api::LivingRef,
        consumed: &mod_api::ItemStack,
        from: [f64; 3],
        target: &mut [f64; 3],
        cancel: &mut bool,
    ) {
        self.fire_item_consumption_teleport(consumer, consumed, from, target, cancel);
    }
    /// A dropped item reaches its lifetime on the server (`ItemEntity.tick`,
    /// NeoForge's `ItemExpireEvent`). Every mod may set or add to `extra_life`,
    /// in ticks, which starts at zero; a positive total keeps the item that much
    /// longer, zero or less lets it expire. Extensions are capped at 32766
    /// ticks.
    pub(crate) fn native_item_expire(
        &mut self,
        item: mod_api::ItemEntityRef,
        extra_life: &mut i32,
    ) {
        self.fire_item_expire(item, extra_life);
    }
    /// A player left-clicks the block `state` at `pos`, `face` the face hit
    /// (`[0, 1, 0]` is the top): where `ServerPlayerGameMode.handleBlockBreakAction`
    /// takes a dig's start, after its reach, spawn protection and world border
    /// checks, before `Block.attack` (a note block plays) or a creative break.
    /// Set `canceled` to ignore the click: the block isn't attacked, dug or
    /// broken on the server, and the player's client gets it back (its own
    /// predicted crack still shows to the clicking player). Server-side, for
    /// every player of the world (the helpers read the acting one).
    pub(crate) fn native_left_click_block(
        &mut self,
        pos: [i32; 3],
        face: [i32; 3],
        state: BlockStateId,
        canceled: &mut bool,
    ) {
        self.fire_left_click_block(pos, face, state, canceled);
    }
    /// `dimension`'s level opened on the server (NeoForge's `LevelEvent.Load`,
    /// posted in `MinecraftServer.createLevels`): before any of its chunks
    /// (`native_chunk_load`), so a mod sets up what it keeps for the level
    /// here. At startup the Overworld, Nether and End are published and
    /// announced in that order, before `server_started`, with each level in
    /// place, and stay open until server shutdown. A custom level closed while
    /// idle fires this again when it reopens.
    /// An explicitly opened runtime level can fire while parked: `dimension`
    /// names that level, not the Host's current one. Once for each open;
    /// `native_level_unload` follows. Server-side.
    pub(crate) fn native_level_load(&mut self, dimension: &str) {
        self.fire_level_load(dimension);
    }
    /// `dimension`'s level closed (NeoForge's `LevelEvent.Unload`, posted after
    /// `level.close()` in `MinecraftServer.stopServer`), after its last save:
    /// drop what the mod kept for it. A custom level closes once nobody has
    /// been in it for 300 ticks, after its chunks' unloads; built-in levels
    /// (Overworld, Nether, End) stay open until server shutdown, unloading
    /// only their unticketed chunks while idle. Every level still open closes
    /// when the server stops, before `WorldCloseFinal`, and when its open or
    /// its loop fails, before `WorldOpenFailed` or `WorldCloseFailed`. Once for
    /// each `native_level_load`. Don't read its blocks. Server-side.
    pub(crate) fn native_level_unload(&mut self, dimension: &str) {
        self.fire_level_unload(dimension);
    }
    /// A mob's, an armor stand's or a connected player's equipment slot took a
    /// different stack (NeoForge's `LivingEquipmentChangeEvent`,
    /// `LivingEntity.collectEquipmentChanges`): `entity` its ID, `kind` its type
    /// (`minecraft:player` for a player), `slot` the `EquipmentSlot`, `from` what
    /// the slot held when last heard and `to` what it holds now. Once per changed
    /// slot, after the mobs' tick; one new to the level counts from empty. A
    /// player's `mainhand` is the selected hotbar slot; their `body` and `saddle`
    /// are never told. The world host's own player is heard by `native_on_equip`.
    /// It only hears. Server-side.
    pub(crate) fn native_living_equipment_change(
        &mut self,
        call: mod_api::__private::LivingEquipmentChangeCall<'_>,
    ) {
        self.fire_living_equipment_change(call);
    }
    /// A server click in a mod menu before the default slot action. A listener sets
    /// `handled` after doing the click itself.
    pub(in crate::game) fn native_menu_clicked(
        &mut self,
        call: mod_api::__private::MenuClickedCall<'_>,
    ) {
        self.fire_menu_clicked(call);
    }
    /// Before `player` picks up orb `orb` in `dimension`. Mutations to `value`
    /// and `count` take effect only when not canceled. Value keeps vanilla
    /// signed integer semantics; count is kept at least one.
    /// Canceling leaves the orb and pickup delay unchanged. Server-side.
    pub(in crate::game) fn native_pickup_xp(
        &mut self,
        player: u128,
        dimension: &str,
        orb: ExperienceOrbRef,
        canceled: &mut bool,
    ) {
        self.fire_pickup_xp(player, dimension, orb, canceled);
    }
    /// A guest's arm swing was handled by the server. `hand` is `mainhand`. Server-side.
    pub(crate) fn native_player_arm_swing(&mut self, player: u128, name: &str, hand: &str) {
        self.fire_player_arm_swing(player, name, hand);
    }
    /// A server player reset their attack strength by swinging (`handlePunch`). The swing and reset
    /// have already happened. The player's state is not swapped in. Server-side.
    pub(crate) fn native_player_attack_cooldown_reset(&mut self, player: u128, name: &str) {
        self.fire_player_attack_cooldown_reset(player, name);
    }
    /// A server player caught a mob in a bucket, after it was removed and the filled bucket was given.
    /// Server-side.
    pub(in crate::game) fn native_player_bucket_entity(
        &mut self,
        player: u128,
        name: &str,
        entity_id: i32,
        entity: &str,
    ) {
        self.fire_player_bucket_entity(player, name, entity_id, entity);
    }
    /// A server player selected a bundle item in `slot` at `index`. The guest's
    /// state is not swapped in during packet handling.
    pub(crate) fn native_player_bundle_item_select(
        &mut self,
        player: u128,
        name: &str,
        slot: i32,
        index: i32,
    ) {
        self.fire_player_bundle_item_select(player, name, slot, index);
    }
    /// A server player changed a beacon's selected effects, after they were stored. Each selected
    /// effect is its load-owned key; None means no effect. Server-side.
    pub(in crate::game) fn native_player_change_beacon_effect(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        primary: Option<EffectKey>,
        secondary: Option<EffectKey>,
    ) {
        self.fire_player_change_beacon_effect(player, name, pos, primary, secondary);
    }
    /// `player` (`name`) changed dimension (NeoForge's
    /// `PlayerChangedDimensionEvent`): `from` and `to` are dimension ids
    /// (`minecraft:overworld`, `minecraft:the_nether`, `minecraft:the_end`).
    /// `ServerPlayer.teleport`: once a portal or command teleport crosses
    /// dimensions and lands, with the destination level and player in place.
    /// `PlayerList.respawn` replacements, including the return after credits,
    /// use `native_player_respawned` instead.
    pub(crate) fn native_player_changed_dimension(
        &mut self,
        player: u128,
        name: &str,
        from: &str,
        to: &str,
    ) {
        self.fire_player_changed_dimension(player, name, from, to);
    }
    /// A server player finished loading the world around them after joining, respawning or changing
    /// dimension. The guest's state is not swapped in. Server-side.
    pub(in crate::game) fn native_player_client_loaded_world(&mut self, player: u128, name: &str) {
        self.fire_player_client_loaded_world(player, name);
    }
    /// A player's data was copied to their new player (NeoForge's
    /// `PlayerEvent.Clone`, from `ServerPlayer.restoreFrom`): `was_death` is
    /// true after a death, false when they left the End after the credits
    /// (`respawn` with `keepAllPlayerData`). `original` reads the old player's
    /// complete data until this hook returns; `player` names the replacement.
    /// Heard before the new player is placed, on a server. Not cancellable.
    pub(crate) fn native_player_clone(
        &mut self,
        player: u128,
        name: &str,
        was_death: bool,
        original: mod_api::OriginalPlayer<'_>,
    ) {
        self.fire_player_clone(player, name, was_death, original);
    }
    /// A command a player typed (NeoForge's `CommandEvent`, before
    /// `Commands.performCommand` runs it), without its slash: any command,
    /// vanilla's and the mods' own, also one the player may not run. Set
    /// `canceled` to stop it: nothing runs and nothing is answered but the
    /// lines pushed to `replies`, which go to the player as system chat.
    /// No player's state is swapped in and the server's player list is busy
    /// with this one, so `native_tell` and the other helpers that take a UUID
    /// reach nobody here: answer through `replies`. Once for each command a
    /// player sends, on a server, and for each line of a quiet command batch
    /// (`sunrise:quiet_commands`, whose replies are dropped). A command the
    /// server refuses before reading it (unsigned chat arguments, the chat
    /// rate limit) never reaches it.
    pub(crate) fn native_player_command(
        &mut self,
        player: u128,
        name: &str,
        command: &str,
        canceled: &mut bool,
        replies: &mut Vec<Text>,
    ) {
        self.fire_player_command(player, name, command, canceled, replies);
    }
    /// A server player has slept for 100 ticks in a bed and now counts as deeply asleep for the
    /// night-skip check.
    pub(crate) fn native_player_deep_sleep(&mut self, player: u128, name: &str) {
        self.fire_player_deep_sleep(player, name);
    }
    /// A server player's book edit or signing packet is about to be applied. `pages` is its page
    /// count; `signing` means the packet has a title. Set `cancel` to drop the edit.
    pub(crate) fn native_player_edit_book(
        &mut self,
        player: u128,
        name: &str,
        pages: i32,
        signing: bool,
        cancel: &mut bool,
    ) {
        self.fire_player_edit_book(player, name, pages, signing, cancel);
    }
    /// A server player threw an egg, after its projectile was launched and the use was applied.
    /// Server-side.
    pub(in crate::game) fn native_player_egg_throw(&mut self, player: u128, name: &str) {
        self.fire_player_egg_throw(player, name);
    }
    /// A server player used a firework rocket to boost while gliding, after the rocket was launched
    /// and consumed. Server-side.
    pub(crate) fn native_player_elytra_boost(&mut self, player: u128, name: &str) {
        self.fire_player_elytra_boost(player, name);
    }
    /// A server player's XP orb pickup cooldown changed to `ticks`. It is
    /// reported after the value changes. Server-side.
    pub(in crate::game) fn native_player_exp_cooldown_change(
        &mut self,
        player: u128,
        name: &str,
        ticks: u32,
    ) {
        self.fire_player_exp_cooldown_change(player, name, ticks);
    }
    /// A server player's fishing rod, with `state`: 0 the cast
    /// (`FishingRodItem.use` throwing the bobber), 1 a fish biting
    /// (`FishingHook.catchingFish`), 2 the reel-in of a bite, which gives the
    /// catch, 3 the reel-in with nothing on the line, 4 the reel-in of a
    /// bobber hooked to an entity or item (like Bukkit's `PlayerFishEvent`;
    /// NeoForge's `ItemFishedEvent` is later, in `FishingHook.retrieve`). Set `cancel` on the cast and nothing
    /// is thrown; on a bite the fish gets away and the wait starts over; on
    /// a reel-in (2 to 4) nothing happens, the bobber stays out and the rod
    /// takes no wear. The bobber ending by itself (too far, the rod put
    /// away, the owner dying), nor the host of a LAN world's rod. The
    /// player's state is swapped in, so `native_acting_player`, `native_tell`
    /// and the item helpers reach them. Once for each cast, bite and reel-in,
    /// on a server.
    pub(in crate::game) fn native_player_fish(
        &mut self,
        player: u128,
        name: &str,
        state: i32,
        cancel: &mut bool,
    ) {
        self.fire_player_fish(player, name, state, cancel);
    }
    /// A server player put a plant into a flower pot, after the potted block state was stored and the
    /// item consumed. `plant` is the item id and `pos` the pot position. Server-side.
    pub(crate) fn native_player_flower_pot_manipulate(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        plant: &str,
    ) {
        self.fire_player_flower_pot_manipulate(player, name, pos, plant);
    }
    /// An eligible connection is waiting for the server's pre-profile login decision.
    pub(in crate::game) fn native_player_handshake(
        &mut self,
        requested_host: &mut String,
        requested_port: u16,
        protocol_version: i32,
        peer_ip: Option<&str>,
        cancel: &mut bool,
        reason: &mut Option<mod_api::Text>,
    ) {
        // Keep the reason typed until it crosses the login packet serialization boundary.
        mod_api::dispatch_player_handshake(
            self,
            requested_host,
            requested_port,
            protocol_version,
            peer_ip,
            reason,
            cancel,
        );
    }
    /// A guest sent the server's current `PlayerInput` flags, after they were applied to the hub. The guest's state is swapped in. Server-side.
    pub(in crate::game) fn native_player_input(&mut self, player: u128, name: &str, flags: i32) {
        self.fire_player_input(player, name, flags);
    }
    /// A server player inserted a book into a lectern, after the block entity and inventory changed.
    /// `book` is the item id. Server-side.
    pub(crate) fn native_player_insert_lectern_book(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        book: &str,
    ) {
        self.fire_player_insert_lectern_book(player, name, pos, book);
    }
    /// A server player changed an item frame after its contents or rotation changed. `action` is
    /// `put`, `rotate`, `remove_item` or `break`; `item` is empty when no item remains, and `rotation`
    /// is the resulting step. Server-side.
    pub(crate) fn native_player_item_frame_change(
        &mut self,
        player: u128,
        name: &str,
        entity_id: u32,
        action: &str,
        item: &str,
        rotation: u8,
    ) {
        self.fire_player_item_frame_change(player, name, entity_id, action, item, rotation);
    }
    /// Mending repaired a server player's item with experience from an orb. `repair` is durability
    /// restored and `experience` is the orb's value. Server-side.
    pub(crate) fn native_player_item_mend(
        &mut self,
        player: u128,
        name: &str,
        item: &str,
        repair: u32,
        experience: u32,
    ) {
        self.fire_player_item_mend(player, name, item, repair, experience);
    }
    /// A network or fake guest was kicked, after the hub recorded the rendered `reason`. The guest's state is
    /// swapped in unless they were already removed in the same poll.
    pub(in crate::game) fn native_player_kicked(&mut self, player: u128, name: &str, reason: &str) {
        self.fire_player_kicked(player, name, reason);
    }
    /// A server player changed the page of a lectern, after its page and redstone pulse changed.
    /// `page` is the zero-based new page. Server-side.
    pub(crate) fn native_player_lectern_page_change(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        page: i32,
    ) {
        self.fire_player_lectern_page_change(player, name, pos, page);
    }
    /// A player was loaded from the world's save (NeoForge's
    /// `PlayerEvent.LoadFromFile`, `PlayerList.loadPlayerData`): their `.dat`
    /// has been read and applied, and they are not placed in a level yet.
    /// `directory` is the world's player directory (`players/data`): a mod
    /// reads its own file there, named `<uuid>.<suffix>` as
    /// `getPlayerFile(suffix)` names it. Heard once for each join, for the
    /// owner of an integrated server as for anyone else, on a server, with no
    /// player's state swapped in. Not cancellable.
    pub(in crate::game) fn native_player_load_from_file(
        &mut self,
        player: u128,
        name: &str,
        directory: &std::path::Path,
    ) {
        self.fire_player_load_from_file(player, name, directory);
    }
    /// `player` (`name`) joined the game (NeoForge's
    /// `PlayerLoggedInEvent`): placed in the world and listed in
    /// `host.players().server_players()`, so `PlayerRef::send_payload` can reach them.
    /// Once a join, with no player's state swapped in. Server-side.
    pub(crate) fn native_player_logged_in(&mut self, player: u128, name: &str) {
        self.fire_player_logged_in(player, name);
    }
    /// A player whose login the server has accepted (NeoForge's
    /// `PlayerNegotiationEvent` stage): the name, whitelist, ban and
    /// server-full checks have passed and the player holds a place, but their
    /// saved state is not loaded and they are not placed in a level yet. Set
    /// `cancel` to refuse them: they are disconnected with `reason` (a plain
    /// "Disconnected" if it is left unset) and nothing else of the join (the
    /// join message, `native_player_logged_in`) happens. Once for each player
    /// who joins, on a server. The player is not placed yet and their client
    /// has no world to show a message in, so no player's state is swapped in
    /// and `native_tell` reaches nobody: the refusal `reason` is the answer.
    /// The checks that need no mod (name, whitelist, bans, a full server,
    /// the proxy's forwarding) refuse earlier, on the connection, and this
    /// hook does not hear those players.
    pub(in crate::game) fn native_player_login(
        &mut self,
        player: u128,
        name: &str,
        cancel: &mut bool,
        reason: &mut Option<Text>,
    ) {
        self.fire_player_login(player, name, cancel, reason);
    }
    /// `player_login_profile`: the mods' edit of an admitted login's properties.
    pub(in crate::game) fn native_player_login_profile(
        &mut self,
        player: u128,
        name: &str,
        properties: &mut Vec<mod_api::AccountProfileProperty>,
    ) {
        self.fire_player_login_profile(player, name, properties);
    }
    /// A server player selected a loom pattern, after the loom menu accepted the button. `pattern` is
    /// the zero-based button index. Server-side.
    pub(in crate::game) fn native_player_loom_pattern_select(
        &mut self,
        player: u128,
        name: &str,
        pattern: i32,
    ) {
        self.fire_player_loom_pattern_select(player, name, pattern);
    }
    /// A server player filled an empty map, after the filled map and its saved data were created.
    /// Server-side.
    pub(crate) fn native_player_map_filled(&mut self, player: u128, name: &str, map_id: i32) {
        self.fire_player_map_filled(player, name, map_id);
    }
    /// A server player named a mob with a name tag, after its name and persistence changed and the tag
    /// was consumed. Server-side.
    pub(in crate::game) fn native_player_name_entity(
        &mut self,
        player: u128,
        name: &str,
        entity_id: u32,
        entity_name: &str,
    ) {
        self.fire_player_name_entity(player, name, entity_id, entity_name);
    }
    /// A server player picked an item from a block or entity, before it enters the hotbar. `pos` is
    /// zero for an entity; `entity_id` is zero for a block. The player's state is swapped in. Set
    /// `cancel` to leave the inventory and client unchanged.
    pub(crate) fn native_player_pick_item(
        &mut self,
        player: u128,
        name: &str,
        from_entity: bool,
        pos: [i32; 3],
        entity_id: i32,
        cancel: &mut bool,
    ) {
        self.fire_player_pick_item(player, name, from_entity, pos, entity_id, cancel);
    }
    /// A server player picked up an arrow after it was removed and its pickup was applied. `item` is
    /// the arrow's pickup item id. Server-side.
    pub(in crate::game) fn native_player_pickup_arrow(
        &mut self,
        player: u128,
        name: &str,
        entity_id: u32,
        item: &str,
    ) {
        self.fire_player_pickup_arrow(player, name, entity_id, item);
    }
    /// A server player's recipe book requested a known recipe be placed in
    /// their crafting grid. `recipe` is the recipe id; `all` is the shift
    /// click form. Server-side.
    pub(crate) fn native_player_recipe_book_click(
        &mut self,
        player: u128,
        name: &str,
        recipe: &str,
        all: bool,
    ) {
        self.fire_player_recipe_book_click(player, name, recipe, all);
    }
    /// A server player's authoritative recipe book flags changed.
    /// Typed identity keeps custom books separate from vanilla ordinals.
    pub(crate) fn native_player_recipe_book_settings_change(
        &mut self,
        player: u128,
        name: &str,
        book: mod_api::RecipeBookIdentity,
        open: bool,
        filtering: bool,
    ) {
        self.fire_player_recipe_book_settings_change(player, name, book, open, filtering);
    }
    /// `PlayerRespawnPositionEvent`: where `player` respawns, once the server found it. The
    /// `original_*` arguments are that point; the mods may change `level`, `position`, `yaw`,
    /// `pitch` and `copy_original_spawn_position` (`game/world/respawn_position.rs` applies them).
    pub(crate) fn native_player_respawn_position(
        &mut self,
        call: mod_api::__private::PlayerRespawnPositionCall<'_>,
    ) {
        self.fire_player_respawn_position(call);
    }
    /// NeoForge's `PlayerRespawnEvent`, after `PlayerList.respawn` placed the new player.
    pub(crate) fn native_player_respawned(
        &mut self,
        player: u128,
        name: &str,
        end_conquered: bool,
    ) {
        self.fire_player_respawned(player, name, end_conquered);
    }
    /// A server player launched with a riptide trident, after the velocity,
    /// spin attack and grounded lift have been applied. The player's state is
    /// swapped in. No reply is needed.
    pub(crate) fn native_player_riptide(&mut self, player: u128, name: &str) {
        self.fire_player_riptide(player, name);
    }
    /// A player shearing a mob is about to do so. Set `cancel` to leave the mob and shears unchanged.
    /// The guest's state is swapped in.
    pub(crate) fn native_player_shear_entity(
        &mut self,
        player: u128,
        name: &str,
        entity_id: i32,
        off_hand: bool,
        cancel: &mut bool,
    ) {
        self.fire_player_shear_entity(player, name, entity_id, off_hand, cancel);
    }
    /// A server player's sprint started or stopped: their client sent
    /// `START_SPRINTING` or `STOP_SPRINTING` (the sprint key, a double tap
    /// forward, or letting go) and the server's player is now `sprinting`
    /// (vanilla's `setSprinting` in `handlePlayerCommand`). Heard only when it
    /// changes. It cannot be stopped, and it hears what the client asks, not
    /// what the server does: a sprint that ends by itself (a hit's knockback,
    /// hunger, a wall) is not reported. The player's state is swapped in, so
    /// the helpers and `native_tell` work. Once for each change, on a server.
    pub(crate) fn native_player_sprint(&mut self, player: u128, name: &str, sprinting: bool) {
        self.fire_player_sprint(player, name, sprinting);
    }
    /// A client started tracking an entity (NeoForge's
    /// `PlayerEvent.StartTracking`, posted at the end of
    /// `ServerEntity.addPairing`): `player` (`name`) has just been sent the
    /// spawn bundle of `entity` (its ID, `uuid` its UUID, `kind` its type,
    /// `minecraft:zombie`) and counts as tracking it. It came into range, its
    /// client's chunks reached it, or it was put in the level; another player
    /// is an entity too, and a player never tracks their own. The entity is
    /// still in the level. What a listener sends the player
    /// (`PlayerRef::send_payload`) reaches it right after that bundle, before
    /// the entity's next changes. Heard in the tick's tracking pass, after the
    /// pass has built every player's packets: the player already counts as
    /// tracking the other entities the same pass paired. The engine is still
    /// walking its tracker: read the game and send payloads, but don't spawn,
    /// remove or teleport entities from here. No player's state is swapped in.
    /// Server-side.
    pub(in crate::game) fn native_player_start_tracking(
        &mut self,
        player: u128,
        name: &str,
        entity: u32,
        uuid: u128,
        kind: &str,
    ) {
        self.fire_player_start_tracking(player, name, entity, uuid, kind);
    }
    /// A server player selected a stonecutter recipe, after the menu accepted
    /// its button. `recipe` is the zero-based button index. Server-side.
    pub(in crate::game) fn native_player_stonecutter_recipe_select(
        &mut self,
        player: u128,
        name: &str,
        recipe: i32,
    ) {
        self.fire_player_stonecutter_recipe_select(player, name, recipe);
    }
    /// A server player stopped spectating `entity_id`, after their camera returned to the player.
    pub(crate) fn native_player_stop_spectating_entity(
        &mut self,
        player: u128,
        name: &str,
        entity_id: i32,
    ) {
        self.fire_player_stop_spectating_entity(player, name, entity_id);
    }
    /// A client stopped tracking an entity (NeoForge's
    /// `PlayerEvent.StopTracking`, posted at the end of
    /// `ServerEntity.removePairing`): `entity` went out of range, left the
    /// level or died, or `player` (`name`) left, changed level or respawned,
    /// which stops everything they tracked. `entity`, `uuid` and `kind` are as
    /// `native_player_start_tracking` told, once for each start; the player no longer counts as tracking
    /// it. A stop of the tracking pass is heard right after the removal
    /// packet, and what a listener sends the player reaches it after that
    /// removal. A leaving player's stops come after `native_player_logged_out`; those
    /// of a respawn or a level change, which send their own removals, at the
    /// next tracking pass. The entity is not necessarily still there, and a
    /// player who left is gone. As in `player_start_tracking`, the engine is
    /// walking its tracker: don't spawn, remove or teleport entities from
    /// here. No player's state is swapped in. Server-side.
    pub(in crate::game) fn native_player_stop_tracking(
        &mut self,
        player: u128,
        name: &str,
        entity: u32,
        uuid: u128,
        kind: &str,
    ) {
        self.fire_player_stop_tracking(player, name, entity, uuid, kind);
    }
    /// A server player swapped their main and off hands, after both inventory slots changed.
    /// Server-side.
    pub(crate) fn native_player_swap_with_equipment_slot(
        &mut self,
        player: u128,
        name: &str,
        slot: &str,
    ) {
        self.fire_player_swap_with_equipment_slot(player, name, slot);
    }
    /// `PlayerSwitchHotbarSlotEvent.Post`: a server player switched from hotbar slot `old_slot`
    /// to `new_slot`, after the selected slot changed and a main-hand item use stopped. The
    /// player's state is not swapped in.
    pub(crate) fn native_player_switch_hotbar_slot_post(
        &mut self,
        player: u128,
        player_entity_id: i32,
        old_slot: i32,
        new_slot: i32,
    ) {
        self.fire_player_switch_hotbar_slot_post(player, player_entity_id, old_slot, new_slot);
    }
    /// `PlayerSwitchHotbarSlotEvent.Pre`: a server player is switching from hotbar slot `old_slot`
    /// to `new_slot` (0 to 8), after the slot range check and before the selected slot changes.
    /// Fired only when the slot differs. The player's state is not swapped in. Set `cancel` to keep
    /// the old slot; the client is sent that slot again, and the post hook doesn't fire.
    pub(crate) fn native_player_switch_hotbar_slot_pre(
        &mut self,
        player: u128,
        player_entity_id: i32,
        old_slot: i32,
        new_slot: i32,
        cancel: &mut bool,
    ) {
        self.fire_player_switch_hotbar_slot_pre(
            player,
            player_entity_id,
            old_slot,
            new_slot,
            cancel,
        );
    }
    /// A server player took the book from a lectern, after the menu and block entity changed. `book`
    /// is the removed item id. Server-side.
    pub(crate) fn native_player_take_lectern_book(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        book: &str,
    ) {
        self.fire_player_take_lectern_book(player, name, pos, book);
    }
    /// A server player is about to travel through an End gateway at `gateway`. Set `cancel` to leave
    /// them at the entrance; the gateway is not consumed and its cooldown does not start. Server-side.
    pub(crate) fn native_player_teleport_end_gateway(
        &mut self,
        player: u128,
        name: &str,
        gateway: [i32; 3],
        cancel: &mut bool,
    ) {
        self.fire_player_teleport_end_gateway(player, name, gateway, cancel);
    }
    /// A server player toggled an ageable mob's age lock with a golden
    /// dandelion, after the lock and item use changed.
    pub(in crate::game) fn native_player_toggle_entity_age_lock(
        &mut self,
        player: u128,
        name: &str,
        entity_id: u32,
        locked: bool,
    ) {
        self.fire_player_toggle_entity_age_lock(player, name, entity_id, locked);
    }
    /// A player's client started (`flying`) or stopped flying (vanilla's
    /// `handlePlayerAbilities`, which honours it only for a player who may
    /// fly: creative or spectator), heard once for each change, on a server,
    /// with the player's state swapped in, so `native_tell` and the other
    /// helpers reach them. The player is already flying or not as asked, as
    /// vanilla leaves them; set `cancel` to refuse it: they keep the state
    /// they had and the server sends their abilities back, as vanilla does
    /// for what it refuses, so the client lands or flies again. Only what a
    /// client asks for is heard: not the flight a game-mode change starts or
    /// ends (`/gamemode`), and not a LAN host's own player.
    pub(crate) fn native_player_toggle_flight(
        &mut self,
        player: u128,
        name: &str,
        flying: bool,
        cancel: &mut bool,
    ) {
        self.fire_player_toggle_flight(player, name, flying, cancel);
    }
    /// A guest's client changed sneaking in its player input packet, after the hub applied the new state.
    /// The guest's state is swapped in. Server-side.
    pub(in crate::game) fn native_player_toggle_sneak(
        &mut self,
        player: u128,
        name: &str,
        sneaking: bool,
    ) {
        self.fire_player_toggle_sneak(player, name, sneaking);
    }
    /// A server applied `velocity` to a player, when syncing changed motion to that player's client.
    pub(crate) fn native_player_velocity(&mut self, player: u128, name: &str, velocity: [f64; 3]) {
        self.fire_player_velocity(player, name, velocity);
    }
    /// A Nether portal was built for a dimension travel, or an End platform
    /// was made for an arrival. `pos` is the lower corner of the Nether
    /// portal rectangle or the End platform origin. The completed blocks are
    /// already in the world.
    pub(in crate::game) fn native_portal_create(&mut self, dimension: &str, pos: [i32; 3]) {
        self.fire_portal_create(dimension, pos);
    }
    /// A thrown splash or lingering potion applied its hit effects.
    pub(in crate::game) fn native_potion_splash(&mut self, item: &str, pos: [f64; 3]) {
        self.fire_potion_splash(item, pos);
    }
    /// A hopper at `pos` began an item transfer attempt. This runs after its
    /// cooldown and enabled checks, before pushing or pulling any items.
    /// Server-side, including the single-player server.
    pub(in crate::game) fn native_prepare_hopper_transfer(&mut self, pos: [i32; 3]) {
        self.fire_prepare_hopper_transfer(pos);
    }
    /// A crafting-grid result was prepared for a server player; `pos` is the crafting table, or None for the inventory grid. Change `result` before it is sent.
    pub(crate) fn native_prepare_item_craft(
        &mut self,
        player: u128,
        name: &str,
        pos: Option<[i32; 3]>,
        result: &mut Option<ItemStack>,
    ) {
        self.fire_prepare_item_craft(player, name, pos, result);
    }
    /// Enchanting offers were calculated for a server player at `pos`. Change `costs` before they are sent to the menu.
    pub(in crate::game) fn native_prepare_item_enchant(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        costs: &mut [i32; 3],
    ) {
        self.fire_prepare_item_enchant(player, name, pos, costs);
    }
    /// A smithing result was prepared for a server player at `pos`. Change it before it is sent.
    pub(in crate::game) fn native_prepare_smithing(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        result: &mut Option<ItemStack>,
    ) {
        self.fire_prepare_smithing(player, name, pos, result);
    }
    /// A projectile hit something (NeoForge's `ProjectileImpactEvent`, posted
    /// where vanilla's projectile has found what it meets, before the hit
    /// acts): `projectile` (its entity id) of type `kind` (`minecraft:arrow`),
    /// shot by `owner` (a mob's or a player's entity id, 0 for none) in the level
    /// `dimension`. It hit the entity `entity` of type `entity_kind` (a player is
    /// `minecraft:player`; a dragon's part is `minecraft:ender_dragon`), or, with
    /// `entity` None, the block at `block` on its `face` (the face's normal,
    /// `[0, 1, 0]` for the top), at the point `at`. Set `canceled` and the impact
    /// does not happen: no damage, no knockback, no sound, no block reaction, the
    /// arrow does not stick, a pearl does not teleport, a potion or a fireball
    /// does not burst. Cancellation alone leaves the projectile alive. Its owner
    /// row is live and a listener can resolve and discard it; a removal prevents
    /// application of the hit independently of cancellation. A flying bobber
    /// skips hooking and redirection to the hit point, but its ordinary move
    /// still stops against solid blocks. Water is not an impact. Bobbers notify
    /// before vanilla's entity deflection and world-border bounce; rockets notify
    /// after deflection and cancellation skips their impact explosion only.
    /// Covers arrows, tridents, thrown items, fireballs, wither skulls, wind
    /// charges, llama spit, shulker bullets, bobbers and rockets. Other families
    /// skip breeze deflections and hits on wind charges. A dragon part or End crystal is told by the id of the
    /// dragon or crystal. Heard only when a mod also sets
    /// `native_hears_projectile_impacts`. Server-side.
    pub(in crate::game) fn native_projectile_impact(
        &mut self,
        call: mod_api::__private::ProjectileImpactCall<'_>,
    ) {
        self.fire_projectile_impact(call);
    }
    /// A raid wave has been spawned at `pos`; `wave` is its one-based wave
    /// number and `raiders` counts the raiders, not the mobs they ride.
    pub(crate) fn native_raid_spawn_wave(
        &mut self,
        raid: i32,
        pos: [i32; 3],
        wave: i32,
        raiders: usize,
    ) {
        self.fire_raid_spawn_wave(raid, pos, wave, raiders);
    }
    /// The commands native mods add to the session's tree: push the complete current set.
    /// Asked at world open before functions compile, on each reload and data pack change,
    /// and after `native_refresh_commands`. No player or hub needs to exist yet; starting
    /// the hub reuses the session registration. Every player's chat box completes them
    /// within their level. A name vanilla has is left out.
    pub(crate) fn native_register_commands(
        &mut self,
        commands: &mut Vec<mod_api::Answered<mod_api::CommandRegistration>>,
    ) {
        self.fire_register_commands(commands);
    }
    /// A server player's client answered a resource pack in play (vanilla's
    /// `ServerboundResourcePackPacket`): `status` is `Action`'s number, 0
    /// successfully loaded, 1 declined, 2 failed download, 3 accepted, 4
    /// downloaded, 5 invalid URL, 6 failed reload, 7 discarded. Heard when the
    /// packet arrives, after the server has handled it; it cannot be stopped.
    /// The player's state is swapped in, so the helpers and `native_tell`
    /// work. The pack a dedicated server offers in the configuration
    /// (`resource-pack` in `server.properties`) is answered before the
    /// player has joined, and those answers are not heard. Once for each
    /// answer, on a server.
    pub(crate) fn native_resource_pack_status(&mut self, player: u128, name: &str, status: i32) {
        self.fire_resource_pack_status(player, name, status);
    }
    /// The server is about to open its worlds: nothing of the world is
    /// loaded yet (NeoForge's `ServerAboutToStartEvent`, "called before the
    /// server begins loading anything"). Vanilla's `MinecraftServer.loadLevel`
    /// start. Server-side: the single player server thread once Play is
    /// pressed, a dedicated server once its settings are read. Each is
    /// followed by `native_server_started` or
    /// `native_world_lifecycle`'s `WorldOpenFailed`. `dedicated` tells a dedicated server from the single
    /// player one.
    pub fn native_server_about_to_start(&mut self, dedicated: bool) {
        self.fire_server_about_to_start(dedicated);
    }
    /// Before play can be accepted: after the dedicated world loads or at
    /// the integrated server's early startup notice.
    pub fn native_server_starting(&mut self, dedicated: bool) {
        self.fire_server_starting(dedicated);
    }
    /// Server teardown has finished, including the final save if possible.
    pub fn native_server_stopped(&mut self, dedicated: bool) {
        self.fire_server_stopped(dedicated);
    }
    /// The server begins teardown (NeoForge's `ServerStoppingEvent`):
    /// the start of vanilla's `MinecraftServer.stopServer`, with the worlds
    /// still loaded and the players still connected and not yet saved, so a
    /// mod can save its own data or tell them. `native_world_lifecycle`'s
    /// `WorldCloseFinal` follows once the worlds are saved. A server whose loop
    /// fails still fires this before the final save attempt and `native_server_stopped`.
    pub fn native_server_stopping(&mut self, dedicated: bool) {
        self.fire_server_stopping(dedicated);
    }
    /// The authoritative server tick's phases: `ServerTickStart` before any
    /// level is ticked (NeoForge's `ServerTickEvent.Pre`), `ServerTickEnd`
    /// after the last (`.Post`). Read blocks with `native_server_block`, by
    /// dimension: which level `self.world` holds here is not defined. A paused
    /// pass fires nothing; a frozen world carries `world_running` false except
    /// during a requested step.
    pub fn native_server_tick(&mut self, context: &LifecycleContext, phase: ServerTickPhase) {
        self.fire_server_tick(context, phase);
    }
    /// A player finished editing a sign: `ServerGamePacketListenerImpl.updateSignText`
    /// has taken their update packet and every check has passed (they were
    /// editing that sign, within reach, the sign is not waxed),
    /// right before the lines go on the sign. `pos` in
    /// `dimension`, the `front` or back side, and the four `lines` as written,
    /// plain text with the formatting codes still in (vanilla strips them
    /// afterwards). Rewrite `lines` (made four lines again) and that is what
    /// the sign gets, or set
    /// `cancel` and the sign keeps its old text. With the player's state
    /// swapped in, so `native_tell` and the helpers that act on the player
    /// reach them. Once for each update a player sends on a server. Not told:
    /// the host's own editing of a sign on a LAN or single-player world
    /// (the editor writes into their world directly), and the sign changes
    /// that are no edit of the text (dyeing, glow ink, waxing, `/data`,
    /// `/setblock`, mods' `native_with_mod_block_entity`).
    pub(crate) fn native_sign_changed(&mut self, call: mod_api::__private::SignChangedCall<'_>) {
        self.fire_sign_changed(call);
    }
    /// Enough players slept through the night in level `dimension` and the clock is about to
    /// move to the morning (NeoForge's `SleepFinishedTimeEvent`). `adjustment` comes in as
    /// `Marker(minecraft:wake_up_from_sleep)` and a mod may set `Absolute`, `Relative` (from
    /// `current`) or another marker; `new_time` is the vanilla morning, read only (contract 2: in
    /// version 1 a mod wrote it). Times are the level's own clock's; resolved once after every mod
    /// (`game/world/wake_time.rs`): a time before `current` or past `i64::MAX`, or a marker the
    /// clock lacks, moves to the morning. `native_time_skip` then hears the jump and may cancel
    /// it. Set `canceled` and the clock stays, and `native_time_skip` is not asked.
    pub(crate) fn native_sleep_finished_time(
        &mut self,
        dimension: &str,
        current: i64,
        new_time: i64,
        adjustment: &mut ClockAdjustment,
        canceled: &mut bool,
    ) {
        self.fire_sleep_finished_time(dimension, current, new_time, adjustment, canceled);
    }

    /// Enough players sleep in level `dimension` to skip the night (`ServerLevel.tick`), before
    /// the clock moves, the sleepers wake and the rain ends. Set `canceled` and none of it
    /// happens this tick.
    pub(in crate::game) fn native_sleep_skip_night(
        &mut self,
        dimension: &str,
        percentage: i32,
        sleeping: i64,
        active: i64,
        canceled: &mut bool,
    ) {
        self.fire_sleep_skip_night(dimension, percentage, sleeping, active, canceled);
    }
    /// A guest asked to complete `command`; `suggestions` may be added to or
    /// filtered. `range` is the replaced byte range.
    pub(crate) fn native_tab_complete(
        &mut self,
        player: u128,
        name: &str,
        command: &str,
        range: [usize; 2],
        suggestions: &mut Vec<Suggestion>,
    ) {
        self.fire_tab_complete(player, name, command, range, suggestions);
    }
    /// A projectile hit the target block at `pos`, which now has `strength`
    /// output power. Server-side.
    pub(crate) fn native_target_hit(&mut self, dimension: &str, pos: [i32; 3], strength: i32) {
        self.fire_target_hit(dimension, pos, strength);
    }
    /// A server player selected trade `index` from `villager` in the trading menu.
    pub(crate) fn native_trade_select(
        &mut self,
        player: u128,
        name: &str,
        villager: Option<u32>,
        index: usize,
    ) {
        self.fire_trade_select(player, name, villager, index);
    }
    /// A vault at `pos` changed to `state`. Its block state is updated.
    /// Server-side.
    pub(crate) fn native_vault_change_state(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        state: &str,
    ) {
        self.fire_vault_change_state(dimension, pos, state);
    }
    /// A vault at `pos` is about to eject a loot item.
    pub(crate) fn native_vault_loot(&mut self, pos: [i32; 3], item: &str) {
        self.fire_vault_loot(pos, item);
    }
    /// A boat or minecart (`kind`, `minecraft:oak_boat`) moved since the last
    /// tick (`VehicleMoveEvent`): `from` and `to` are its positions, heard
    /// after the tick moved it. Once a tick for each vehicle that carries a
    /// server player, whose state is then swapped in (as for
    /// a command executor: `native_acting_player`, `native_tell` and
    /// the item helpers reach them; `player` is their UUID), and for one a
    /// server player rode and left, until it has stood still for ten
    /// seconds (`player` is none, nobody is swapped in). It only hears.
    /// Cheap when nothing moves: one position compare a tick for each watched
    /// vehicle. Not heard: a vehicle moved before anyone rode it, a mount
    /// (horse, pig...) or a vehicle carrying only a mob. Server-side, in the
    /// level the vehicle is in.
    pub(crate) fn native_vehicle_moved(
        &mut self,
        player: Option<u128>,
        kind: &str,
        from: [f64; 3],
        to: [f64; 3],
    ) {
        self.fire_vehicle_moved(player, kind, from, to);
    }
    /// A timed world-border size change finished on the server, after its
    /// final size was applied. Server-side, including the single-player
    /// server.
    pub(crate) fn native_world_border_bounds_change_finish(&mut self, level: LevelRef) {
        self.fire_world_border_bounds_change_finish(level);
    }

    /// An anvil at `pos` took damage, changing its block from `from` to `to`.
    /// `to` is air when the anvil broke. Server-side.
    pub(in crate::game) fn native_anvil_damaged(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        from: &str,
        to: &str,
    ) {
        self.fire_anvil_damaged(dimension, pos, from, to);
    }
    /// A dispenser equipped `item` on the first eligible `target` in front
    /// of its block at `pos`. `target` is `player`, `mob` or `armor_stand`.
    /// The equip, item consumption, sound and game event have completed.
    /// Server-side.
    pub(in crate::game) fn native_block_dispense_armor(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        item: &str,
        target: &str,
    ) {
        self.fire_block_dispense_armor(dimension, pos, item, target);
    }
    /// A dispenser at `pos` failed to dispense `item`. Server-side.
    pub(in crate::game) fn native_block_failed_dispense(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        item: &str,
    ) {
        self.fire_block_failed_dispense(dimension, pos, item);
    }
    /// A guest's block inventory lock check at `pos`. `locked` is the
    /// vanilla result and may be changed. The guest's state is not swapped in.
    pub(in crate::game) fn native_block_lock_check(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        locked: &mut bool,
    ) {
        self.fire_block_lock_check(player, name, pos, locked);
    }
    /// A dispenser sheared `entity` of kind `kind` at the block in front of
    /// its own block at `pos`, after the shearing effects. Server-side.
    pub(in crate::game) fn native_block_shear_entity(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        entity: u32,
        kind: &str,
    ) {
        self.fire_block_shear_entity(dimension, pos, entity, kind);
    }
    /// A custom entry of the effective consumable, with its pre-shrink stack
    /// and the living entity consuming it. Unclaimed kinds do nothing.
    pub fn native_consume_effect(
        &mut self,
        effect: &sunrise_items::items::NativeConsumeEffect,
        stack: &ItemStack,
        user: mod_api::EntityRef,
    ) -> bool {
        use sunrise_gameplay_common::game::api_values::TryIntoApi;

        let Some(binding) = effect.binding else {
            log::warn!("consume effect {} has no registered callback", effect.kind);
            return false;
        };
        let stack = match stack.try_into_api() {
            Ok(stack) => stack,
            Err(error) => {
                log::warn!("consume effect {}: stack refused: {error:?}", effect.kind);
                return false;
            }
        };
        mod_api::dispatch_consume_effect(self, &effect.kind, &effect.data, &stack, user, binding)
    }
    /// A player opens one of the game's own menus (`ServerPlayer.openMenu`; the
    /// job of NeoForge's `PlayerContainerEvent.Open`, which comes after the
    /// menu is open and cannot stop it): `kind` is the menu type
    /// (`minecraft:generic_9x3`, `minecraft:furnace`, `minecraft:crafting`,
    /// `minecraft:merchant`, `minecraft:anvil`, `minecraft:lectern`...; a
    /// horse's inventory is `minecraft:horse_inventory`) and `pos` the block
    /// it belongs to, none for a villager, a mount or a vehicle's chest.
    /// Heard after the checks that refuse the menu (a locked container, a
    /// spectator) and before the menu that was open closes. Set `cancel` and
    /// nothing opens: the open menu stays as it was and the click is spent.
    /// The player's state is swapped in for the call, as for
    /// a command executor, so `native_acting_player`, `native_tell` and
    /// the item helpers reach them. Not heard: a menu a mod opens
    /// (`open_menu`, unlike `open_mod_menu`), the inventory screen, and the editors (sign,
    /// book, command block). Server-side, for every player of the world.
    pub(in crate::game) fn native_container_opened(
        &mut self,
        player: u128,
        name: &str,
        kind: &str,
        pos: Option<[i32; 3]>,
        cancel: &mut bool,
    ) {
        self.fire_container_opened(player, name, kind, pos, cancel);
    }
    /// A crafter at `pos` completed recipe `recipe`.
    pub(in crate::game) fn native_crafter_craft(&mut self, pos: [i32; 3], recipe: &str) {
        self.fire_crafter_craft(pos, recipe);
    }
    /// The server is assembling its data tables, as for
    /// `native_data_pack_files` (a world's first open and every `/reload`):
    /// push the data map types your mod registers (NeoForge's
    /// `RegisterDataMapTypesEvent`). A type is a namespaced `id`
    /// (`namespace:path`) for a `registry`: any the game names. Its values come
    /// from the world's data packs'
    /// `data/<namespace>/data_maps/<registry folder>/<path>.json`, and from
    /// files you push through `native_data_pack_files` under the key
    /// `data_maps/<namespace>:<registry folder>/<path>`. Each file has a `values`
    /// object (entry IDs and `#tags` to JSON), optional boolean `replace` and
    /// optional `remove` list. Entry conditions run before reading values and
    /// malformed layers are skipped whole. Push a typed map's `DataMap::ty`
    /// and read its values with `ItemKey::data` and `BlockKey::data`. Server-side,
    /// not synced to clients.
    pub(in crate::game) fn native_data_map_types(&mut self, types: &mut Vec<DataMapType>) {
        self.fire_data_map_types(types);
    }
    /// The folders of JSON files native mods read from the data
    /// (`AddServerReloadListenersEvent`): push yours, each a listener id and a
    /// folder under `data/<namespace>/`. Asked at the start of every data load
    /// of a server: a world opening, `/reload`. A folder that leaves
    /// The server is assembling its data tables from the world's data packs
    /// (`ReloadableServerResources`): a world is opening, or `/reload` was
    /// run, and the recipes, tags, loot and advancements are built after
    /// this. Push `(path, bytes)` pack files and they join this load. `path`
    /// is the overlay's key, `<kind>/<namespace>:<path>` without `.json`, over
    /// the kinds the tables read: `recipe/mymod:thing`,
    /// `tags/mymod:block/thing`, `loot_table/…`, `advancement/…`,
    /// `trade_set/…`, `villager_trade/…`, `dialog/…`. A file that isn't JSON
    /// is skipped with a warning. Asked once per load on both paths, so this
    /// is where a mod recomputes them.
    /// Contributed files sit below the world's own data packs: a pack's file
    /// for the same path still overrides a mod's. Server-side (the single
    /// player's integrated server included).
    pub(in crate::game) fn native_data_pack_files(&mut self, files: &mut Vec<(String, Vec<u8>)>) {
        self.fire_data_pack_files(files);
    }
    /// After admission, collect once before building and sending this player's recipes.
    /// Reload asks once per online player before their recipes are re-sent; none when empty.
    pub(in crate::game) fn native_datapack_sync(
        &mut self,
        call: mod_api::__private::DatapackSyncCall<'_>,
    ) {
        self.fire_datapack_sync(call);
    }
    /// The server's difficulty changed, from `from` to `to` (`peaceful`,
    /// `easy`, `normal`, `hard`; NeoForge's `DifficultyChangeEvent`). It is
    /// vanilla's `MinecraftServer.setDifficulty`, which `/difficulty`, a
    /// dedicated server's management and a test environment's difficulty all
    /// go through, and it is told only when the difficulty is another one. It
    /// only hears: the change has already happened. Server-side.
    pub(in crate::game) fn native_difficulty_changed(&mut self, from: &str, to: &str) {
        self.fire_difficulty_changed(from, to);
    }
    /// `DispenserBlock.getDispenseMethod`: choose a behaviour for any item,
    /// including vanilla items, on the authoritative server (not droppers).
    /// The block the dispenser faces is asked first (`dispense_method`), and
    /// its answer keeps this part from being asked. Leave answer None for
    /// vanilla. Otherwise answer `(remainder, event)`:
    /// a remainder of None empties the slot, `Some(stack)` replaces it (the hook
    /// owns consumption: no extra item is removed afterwards). `event` is the
    /// level event the dispenser plays with its smoke: 1000 the click, 1001 the
    /// fail sound (`OptionalDispenseItemBehavior` without success), or 0 for
    /// nothing at all (`DispenseItemBehavior.NOOP`). To drop one normally,
    /// answer `(dispense_default(..), 1000)`.
    pub fn native_dispense_item(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        facing: Direction,
        stack: &ItemStack,
        answer: &mut Option<mod_api::Dispensed>,
    ) {
        self.fire_dispense_item(dimension, pos, facing, stack, answer);
    }
    /// Entities from `chunk` finished loading into the server world. `count`
    /// is the number of saved entity records read.
    pub(in crate::game) fn native_entities_load(
        &mut self,
        dimension: &str,
        chunk: [i32; 2],
        count: usize,
    ) {
        self.fire_entities_load(dimension, chunk, count);
    }
    /// An entity leaves the server level `dimension` (NeoForge's
    /// `EntityLeaveLevelEvent`): vanilla's `onTrackingEnd`, when it is removed
    /// for any reason (picked up, merged, despawned, burnt, blown up, killed,
    /// taken by a hopper or a mob, gone through a portal) or its chunk unloads,
    /// a level closing with nobody in it included. Not for the entities still
    /// loaded when the server stops, as in vanilla. `entity` is its ID and
    /// `kind` its type (`minecraft:item`); it is no longer in the level's lists.
    /// It only hears. 53 told item entities, experience orbs and mobs; it now
    /// tells every kind `native_entity_join_level` does (`kind` is the type), each
    /// once, when the game next notices it gone (the tick or action that removed
    /// it). Check `kind`. Server-side.
    pub(in crate::game) fn native_entity_leave_level(
        &mut self,
        dimension: &str,
        entity: u32,
        kind: &str,
    ) {
        self.fire_entity_leave_level(dimension, entity, kind);
    }
    /// An entity mounts or dismounts a vehicle (NeoForge's `EntityMountEvent`, posted from
    /// `Entity.startRiding` and `removeVehicle`): `entity` of type `kind` (`minecraft:player`,
    /// `minecraft:zombie`) in the server level `dimension`, getting on (`mounting`) or off the
    /// `vehicle` of type `vehicle_kind` (`minecraft:oak_boat`, `minecraft:minecart`,
    /// `minecraft:horse`, `minecraft:cushion`). Set `canceled` and it does not: a player stays
    /// where they were, a mob does not board, a `/ride` fails, and a rider whose dismount is
    /// canceled keeps riding, including during vehicle destruction. A removed
    /// boat or cart is no longer a valid handle. Explicit later dismounts notify
    /// again; resident riders also retry in entity-ticking chunks, while rejected
    /// passengers do not tick. Loading a save does not notify. Server-side.
    pub(in crate::game) fn native_entity_mount(
        &mut self,
        call: mod_api::__private::EntityMountCall<'_>,
    ) {
        self.fire_entity_mount(call);
    }
    /// A server entity was moved by a same-level teleport command, after its
    /// position changed. `entity` is its network id; `from` and `to` are
    /// coordinates in `dimension`. This hook reports command teleports that
    /// use Sunrise's same-level target path; it does not change vanilla's
    /// movement.
    pub(in crate::game) fn native_entity_teleport(
        &mut self,
        dimension: &str,
        entity: u32,
        from: [f64; 3],
        to: [f64; 3],
    ) {
        self.fire_entity_teleport(dimension, entity, from, to);
    }
    /// A leashed mob's leash was removed, before any lead drop is handled
    /// (Bukkit's `EntityUnleashEvent`).
    pub(in crate::game) fn native_entity_unleash(&mut self, entity: u32, kind: &str) {
        self.fire_entity_unleash(entity, kind);
    }
    /// A server explosion is about to begin at `center`, with `radius` (Bukkit's
    /// `ExplosionPrimeEvent`). This is listen-only: the explosion proceeds as
    /// usual. Server-side, including the single-player server.
    pub(in crate::game) fn native_explosion_prime(&mut self, center: [f64; 3], radius: f32) {
        self.fire_explosion_prime(center, radius);
    }
    /// The food level of the server player `player` (`name`) is about to change
    /// from `from` to `to` (Bukkit's `FoodLevelChangeEvent`):
    /// eating, hunger and saturation effects, the exhaustion that costs a
    /// point, starving. Change `to` (kept between 0 and 20; saturation never
    /// exceeds it) or set `cancel` to leave the level at `from`, saturation
    /// included. Heard once for the change a player's turn made as a whole, so
    /// two changes in one tick arrive as one. Not heard: the full food bar a
    /// respawn gives, the food a player joins with and the host of a LAN
    /// world. The player's state is swapped in, so `native_acting_player`,
    /// `native_tell` and the item helpers reach them. On a server.
    pub(in crate::game) fn native_food_level_changed(
        &mut self,
        player: u128,
        name: &str,
        from: i32,
        to: &mut i32,
        cancel: &mut bool,
    ) {
        self.fire_food_level_changed(player, name, from, to, cancel);
    }
    /// An item frame (`minecraft:item_frame`, `minecraft:glow_item_frame`) or
    /// a painting (`minecraft:painting`), `kind` its entity type and `pos` the
    /// block it hangs in, is about to break: a server player's hit
    /// (`player`, their state swapped in, so `native_tell` answers them), a
    /// blast, or the wall it hangs on gone (`player` None for both). Where
    /// `BlockAttachedEntity.hurtServer` and its survival check would kill
    /// it, before anything drops or sounds. Set `cancel` and it stays, as it
    /// was. A hit that only takes the item out of a frame does not break the
    /// frame and is not asked. The game's own player's hit is not asked, nor
    /// is `/kill`.
    pub(in crate::game) fn native_hanging_break(
        &mut self,
        player: Option<u128>,
        kind: &str,
        pos: [i32; 3],
        cancel: &mut bool,
    ) {
        self.fire_hanging_break(player, kind, pos, cancel);
    }
    /// A server player is about to open an inventory supplied by a block at
    /// `pos`. Set `cancel` to leave the inventory closed. The player's state
    /// is not swapped in.
    pub(in crate::game) fn native_inventory_block_start(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        cancel: &mut bool,
    ) {
        self.fire_inventory_block_start(player, name, pos, cancel);
    }
    /// A dropped item entity was successfully added to the server level.
    pub(in crate::game) fn native_item_spawn(
        &mut self,
        entity: u32,
        item: &str,
        count: i32,
        pos: [f64; 3],
    ) {
        self.fire_item_spawn(entity, item, count, pos);
    }
    /// A server lightning bolt is about to be created. `cause` is `weather`,
    /// `trident`, `command` or `mod` (a native mod's `strike_lightning`); `player` is the responsible player when known.
    /// Set `cancel` to prevent the bolt and its effects.
    pub(in crate::game) fn native_lightning_strike(
        &mut self,
        player: Option<u128>,
        pos: [i32; 3],
        cause: &str,
        cancel: &mut bool,
    ) {
        self.fire_lightning_strike(player, pos, cause, cancel);
    }
    /// A block broke or a mob died, and `drops` are the items about to drop, in
    /// the order they drop: what the loot tables rolled (after
    /// `native_loot_table_load`'s changes), and for a mob what it wore, held or
    /// carried (a fox's mouth item, an allay's, piglin's or horse's inventory),
    /// but not an enderman's carried block or a copper golem's preserved
    /// equipment, or custom drops carrying item-entity state such as an initial
    /// age, which join the later living-drops collection instead.
    /// A charged creeper's head and a fox's mouth
    /// item come before the mob's own table. A block's stacks get its block
    /// entity's data (a shulker box's items) after this. Add, change or take out
    /// stacks; empty ones don't drop (NeoForge's global loot modifiers, Fabric's
    /// `LootTableEvents.MODIFY_DROPS`); `context` says what dropped them and
    /// how, and it is called even when `drops` is empty. A player's use drops
    /// through it too (`LootDrops::used`: shears, a brush, an item
    /// frame), with that player named. It hears blocks that a
    /// player, an explosion or a mob breaks, not other breaks (leaves decaying,
    /// a torch losing its wall, a piston, `/setblock ... destroy`), nor a
    /// creative break or a tool that can't harvest the block, nor experience.
    /// Server-side, for every player.
    pub(in crate::game) fn native_loot_drops(
        &mut self,
        context: &LootDrops<'_>,
        drops: &mut Vec<ItemStack>,
    ) {
        self.fire_loot_drops(context, drops);
    }
    /// The number of players with the mod container at `pos` of `dimension`
    /// open went from `previous` to `viewers` (vanilla's
    /// `ContainerOpenersCounter`, as a barrel keeps it): 0 to more than 0 is
    /// `onOpen`, back to 0 is `onClose`, any change `openerCountChanged`, so a
    /// block can show itself open while `viewers` is not 0. Synchronous with
    /// non-spectator menu opens and closes, with that player's state in place;
    /// scheduled rechecks run every five ticks while open and repair lost
    /// openers. Also called by a recheck when the count is unchanged. Server-side.
    pub(in crate::game) fn native_mod_container_viewers(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        viewers: u32,
        previous: u32,
    ) {
        self.fire_mod_container_viewers(dimension, pos, viewers, previous);
    }
    /// A server player completed an advancement, after progress was recorded.
    pub(in crate::game) fn native_player_advancement_done(
        &mut self,
        player: u128,
        name: &str,
        advancement: &str,
    ) {
        self.fire_player_advancement_done(player, name, advancement);
    }
    /// A criterion of a server player's advancement was granted or revoked, after its progress changed
    /// and before any completion rewards. A grant or revoke that changes nothing is not heard.
    /// The call carries a snapshot of all criterion dates and requirement results at this mutation;
    /// no player-progress storage is borrowed while listeners run.
    pub(in crate::game) fn native_player_advancement_progress(
        &mut self,
        call: mod_api::__private::PlayerAdvancementProgressCall<'_>,
    ) {
        self.fire_player_advancement_progress(call);
    }
    /// A server player is about to enter a bed after the sleep checks pass. Set `cancel` to keep them
    /// out. The player's state is swapped in.
    pub(in crate::game) fn native_player_bed_enter(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        cancel: &mut bool,
    ) {
        self.fire_player_bed_enter(player, name, pos, cancel);
    }
    /// The command tree a server sends a player (NeoForge's and Paper's
    /// "commands sent to a player" stage): when they join and when their
    /// level changes (`/op`, `/deop`, the LAN settings). `names` are the
    /// root commands their client would get, vanilla's they may use and the
    /// mods'; take a name out and that client gets the tree without it
    /// (and without what it leads to), so it neither completes nor
    /// highlights it. This hides the command from the client only: the
    /// server still runs a typed one, `native_player_command` stops it.
    /// For a player who is already on, their state is swapped in and the
    /// helpers (`native_tell`...) work; for one who is joining it is not, and
    /// their client has no world to tell yet. The tree the hub sends again by
    /// itself (after a respawn or a change of level) keeps the last answer
    /// and does not ask again; the integrated server's owner is not asked
    /// when the world's cheats are switched.
    pub(in crate::game) fn native_player_commands_sent(
        &self,
        call: mod_api::__private::PlayerCommandsSentCall<'_>,
    ) {
        self.fire_player_commands_sent(call);
    }
    /// `player` (`name`) closed the menu `native_player_container_open` told
    /// of (NeoForge's `PlayerContainerEvent.Close`, posted in
    /// `ServerPlayer.doCloseContainer` after the menu's `removed`): they
    /// closed it, walked out of reach, died, changed level, or another menu
    /// opened. A player who leaves closes it before `native_player_logged_out`
    /// (vanilla posts nothing for a disconnect; Sunrise does, so each open has
    /// its close). Once for each open; not told when the server stops. The
    /// menu's items are where it left them: the block's, and the cursor stack
    /// back in the inventory. State swapped in, when the player has a body.
    /// Server-side.
    pub(in crate::game) fn native_player_container_close(
        &mut self,
        player: PlayerRef,
        menu_type: &str,
        pos: Option<[i32; 3]>,
        menu: Option<mod_api::MenuRef>,
    ) {
        self.fire_player_container_close(player, menu_type, pos, menu);
    }
    /// `player` (`name`) opened a menu (NeoForge's `PlayerContainerEvent.Open`,
    /// posted when `ServerPlayer.openMenu` sets the new `containerMenu`): any
    /// menu, vanilla or a mod's. `menu_type` is the menu type's name
    /// (`minecraft:generic_9x3`, `minecraft:furnace`, `minecraft:merchant`,
    /// `minecraft:crafting`, `minecraft:lectern`, a mod's own), `pos` the
    /// block it is of in `dimension` (none for a trade, a vehicle's, a mod's
    /// native menu or a mount's inventory, which has no menu type in vanilla
    /// and is told as `sunrise:mount_inventory`). The player's own inventory
    /// is not one, nor is failed construction. Heard synchronously after construction, the open packet
    /// is queued and full contents sent, with the new menu already current.
    /// Replacing a menu follows its close; even inside one tick, open precedes
    /// `native_player_container_close`. The player's state is swapped in. Server-side.
    pub(in crate::game) fn native_player_container_open(
        &mut self,
        player: u128,
        name: &str,
        dimension: &str,
        menu_type: &str,
        pos: Option<[i32; 3]>,
        menu: Option<mod_api::MenuRef>,
    ) {
        self.fire_player_container_open(player, name, dimension, menu_type, pos, menu);
    }
    /// An item cooldown is about to start for a server player. `item` is the item id and `ticks` its
    /// duration. The player's state is not swapped in. Set `cancel` to skip starting the cooldown.
    /// Server-side.
    pub(in crate::game) fn native_player_item_cooldown(
        &mut self,
        player: u128,
        name: &str,
        item: &str,
        ticks: i32,
        cancel: &mut bool,
    ) {
        self.fire_player_item_cooldown(player, name, item, ticks, cancel);
    }
    /// A server player's experience level changed, after the new level was applied.
    pub(in crate::game) fn native_player_level_change(
        &mut self,
        player: u128,
        name: &str,
        from: i32,
        to: i32,
    ) {
        self.fire_player_level_change(player, name, from, to);
    }
    /// `player` (`name`) left the game (NeoForge's `PlayerLoggedOutEvent`), by
    /// any path: a lost connection, a kick, the server stopping or failing.
    /// Once for each player `native_player_logged_in` told of, and also for a
    /// kicked fake without a preceding login. Silent `remove_fake` sends no logout.
    /// The player is still in `native_server_players` when the hook fires. A
    /// listener's edits persist only on leaves this server saves. A completed proxy
    /// hand-over or a crash writes no logout save. Only when the server stops can
    /// a player still get a last message. A kicked fake has already left the list
    /// and is not saved.
    /// Server-side.
    pub fn native_player_logged_out(&mut self, player: u128, name: &str) {
        self.fire_player_logged_out(player, name);
    }
    /// A server player opened a sign for editing, after the editor state was made. `front` selects the
    /// front or back side. Server-side.
    pub(in crate::game) fn native_player_open_sign(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        front: bool,
    ) {
        self.fire_player_open_sign(player, name, pos, front);
    }
    /// A recipe is about to be unlocked for a server player. Set `cancel` to keep it locked. The
    /// player's state is not swapped in. Server-side.
    pub(in crate::game) fn native_player_recipe_discovered(
        &mut self,
        player: u128,
        name: &str,
        recipe: &str,
        cancel: &mut bool,
    ) {
        self.fire_player_recipe_discovered(player, name, recipe, cancel);
    }
    /// A player was saved to the world's store (NeoForge's
    /// `PlayerEvent.SaveToFile`, `PlayerList.save`): when they leave or the
    /// world closes, when a dedicated or integrated server saves its guests,
    /// and for an integrated server's owner when the world saves them.
    /// Once for each save. `directory`
    /// is the world's player directory (`players/data`): write your own file
    /// there, named `<uuid>.<suffix>`, never the player's `.dat`. The player
    /// may be leaving, so no player's state is swapped in. Not cancellable.
    /// Autosave snapshots are written asynchronously: this notification runs
    /// on a later server tick after a successful write, possibly after WorldSaved.
    /// Live state may have advanced beyond the saved snapshot; a process exit
    /// before completions are drained can prevent notification delivery.
    pub(in crate::game) fn native_player_save_to_file(
        &mut self,
        player: u128,
        name: &str,
        directory: &std::path::Path,
    ) {
        self.fire_player_save_to_file(player, name, directory);
    }
    /// A server player's shield was disabled by a blocked melee hit. The
    /// shield cooldown is set immediately after this call. Helpers looking up
    /// the guest through `self.lan` may find nothing during packet handling.
    /// Server-side. 276.
    pub(in crate::game) fn native_player_shield_disable(
        &mut self,
        player: u128,
        name: &str,
        ticks: u32,
    ) {
        self.fire_player_shield_disable(player, name, ticks);
    }
    /// A statistic is about to be awarded to a server player, before its count and scoreboard criteria
    /// change. `stat_type` and `stat` are ids such as `minecraft:mined` and `minecraft:stone`, and
    /// `value` the amount; all three are writable. Invalid rewrites retain the original. Set `cancel` to award nothing.
    pub(in crate::game) fn native_player_stat_award(
        &mut self,
        player: u128,
        name: &str,
        stat_type: &mut String,
        stat: &mut String,
        value: &mut i32,
        cancel: &mut bool,
    ) {
        self.fire_player_stat_award(player, name, stat_type, stat, value, cancel);
    }
    /// A server player a command teleported (NeoForge's
    /// `EntityTeleportEvent`, for a player): `/tp` and `/teleport`, inside
    /// their level or into another one. `from` and `to` are their feet before
    /// and after, `dimension` the level they are in now. Heard after the move
    /// has reached the player's state and their client (an inter-level
    /// teleport: when the new level is entered, before it has loaded around
    /// them), so it cannot stop it. The player's state is swapped in, so the
    /// helpers that read the acting player and `native_tell` work. Once for
    /// each teleported player, on a server. Ender pearls and chorus fruit
    /// have their own hooks; a spectator's teleport menu is not heard yet;
    /// portals and respawns are not teleports here.
    pub(in crate::game) fn native_player_teleported(
        &mut self,
        player: u128,
        name: &str,
        from: [f64; 3],
        to: [f64; 3],
        dimension: &str,
    ) {
        self.fire_player_teleported(player, name, from, to, dimension);
    }
    /// A server player removed an entity's leash, after it was cleared.
    pub(in crate::game) fn native_player_unleash_entity(
        &mut self,
        player: u128,
        name: &str,
        entity_id: u32,
    ) {
        self.fire_player_unleash_entity(player, name, entity_id);
    }
    /// A raid at `pos` changed to `state` (`triggered`, `victory`, `loss` or
    /// `stopped`). The handle identifies its owning level, even if parked.
    pub(in crate::game) fn native_raid_state_change(
        &mut self,
        raid: mod_api::RaidRef,
        pos: [i32; 3],
        state: &str,
    ) {
        self.fire_raid_state_change(raid, pos, state);
    }
    /// A sculk catalyst at `pos` has begun blooming after taking experience.
    /// The catalyst block state has changed. Server-side.
    pub(in crate::game) fn native_sculk_bloom(&mut self, dimension: &str, pos: [i32; 3]) {
        self.fire_sculk_bloom(dimension, pos);
    }
    /// The server opened its worlds and is about to tick (NeoForge's
    /// `ServerStartedEvent`): every level loaded, no player joined. Where
    /// `WorldOpenReady` fired: the end of the open, before the tick loop.
    pub fn native_server_started(&mut self, dedicated: bool) {
        self.fire_server_started(dedicated);
    }
    /// The world's spawn point changed to `pos` in level `dimension`, facing `angle`.
    pub(in crate::game) fn native_spawn_change(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        angle: [f32; 2],
    ) {
        self.fire_spawn_change(dimension, pos, angle);
    }
    /// The overworld clock is about to jump, from sleep or `/time`. Set
    /// `cancel` to keep `from`.
    pub(in crate::game) fn native_time_skip(
        &mut self,
        dimension: &str,
        from: i64,
        to: i64,
        cause: &str,
        cancel: &mut bool,
    ) {
        self.fire_time_skip(dimension, from, to, cause, cancel);
    }
    /// TNT at `pos` is about to be lit (primed): a flint and steel or a fire
    /// charge on it, fire burning it, a redstone signal, an arrow, or a
    /// dispenser's flint and steel or TNT item. `player` is the server player
    /// whose click lit it, with their state swapped in (so `native_tell`
    /// answers them), None when no player is behind it (fire, redstone, an
    /// arrow, a dispenser). Set `cancel` and it is not primed: a TNT block
    /// stays where it is (one that fire or a signal had already taken out is
    /// put back, without a block update), a dispenser's TNT item stays in it.
    /// Not asked: TNT set off by a blast (`native_explosion_detonate` spares
    /// blocks), a TNT minecart and a sulfur cube. Server-side and in single
    /// player.
    pub(in crate::game) fn native_tnt_primed(
        &mut self,
        player: Option<u128>,
        pos: [i32; 3],
        cancel: &mut bool,
    ) {
        self.fire_tnt_primed(player, pos, cancel);
    }
    /// Non-cancellable collect for one villager or wandering-trader take. True when a
    /// validated answer changed the offer; observing alone leaves the input untouched.
    pub(in crate::game) fn native_trade_with_villager(
        &mut self,
        merchant: u32,
        kind: &str,
        offer: &mut MerchantOffer,
    ) -> Result<bool, sunrise_gameplay_common::game::transactions::decision::InvalidReason> {
        use sunrise_gameplay_common::game::api_values::{
            OfferWriteError, TryIntoApi, checked_merchant_offer,
        };
        use sunrise_gameplay_common::game::transactions::decision::InvalidReason;
        if !mod_api::__private::listened(mod_api::HookId::TradeWithVillager) {
            return Ok(false);
        }
        let original = offer.try_into_api().map_err(|_| InvalidReason::Plan)?;
        let mut answer = original.clone();
        mod_api::dispatch_trade_with_villager(self, merchant, kind, &mut answer);
        if answer == original
            && answer.price_multiplier.to_bits() == original.price_multiplier.to_bits()
        {
            return Ok(false);
        }
        *offer = checked_merchant_offer(&answer, self.items()).map_err(|reason| {
            log::warn!("trade_with_villager: invalid offer answer: {reason:?}");
            match reason {
                OfferWriteError::NonFinitePrice => InvalidReason::Numeric,
                OfferWriteError::Stack(_) => InvalidReason::Plan,
            }
        })?;
        Ok(true)
    }
    /// A boat or minecart was created at `pos`, after it joined the level.
    /// `kind` is the vehicle type. Server-side.
    pub(in crate::game) fn native_vehicle_create(&mut self, kind: &str, pos: [f64; 3]) {
        self.fire_vehicle_create(kind, pos);
    }
    /// A vehicle is about to take damage from a player's attack. `kind` is
    /// `boat` or the minecart type; mods may change `damage` or set `cancel`
    /// to leave its damage and wobble unchanged. The attack itself still
    /// completes. Server-side.
    pub(in crate::game) fn native_vehicle_damage(
        &mut self,
        kind: &str,
        damage: &mut f32,
        cancel: &mut bool,
    ) {
        self.fire_vehicle_damage(kind, damage, cancel);
    }
    /// A boat or minecart is about to be destroyed at `pos`. TNT minecarts
    /// that prime instead of breaking are not heard. Server-side.
    pub(in crate::game) fn native_vehicle_destroy(&mut self, kind: &str, pos: [f64; 3]) {
        self.fire_vehicle_destroy(kind, pos);
    }
    /// The server changed its weather parameters. `raining` and `thundering`
    /// are the requested final states. Set `cancel` to keep the previous
    /// weather parameters. Server-side.
    pub(in crate::game) fn native_weather_change(
        &mut self,
        dimension: &str,
        clear_ticks: i32,
        weather_ticks: i32,
        raining: bool,
        thundering: bool,
        cancel: &mut bool,
    ) {
        self.fire_weather_change(
            dimension,
            clear_ticks,
            weather_ticks,
            raining,
            thundering,
            cancel,
        );
    }
    /// The dedicated server's whitelist changed to `enabled`. 305.
    pub(in crate::game) fn native_whitelist_state_update(&mut self, enabled: bool) {
        self.fire_whitelist_state_update(enabled);
    }
    /// The server world's border bounds changed, either immediately or when
    /// its timed size change was requested.
    pub(in crate::game) fn native_world_border_bounds_changed(
        &mut self,
        level: LevelRef,
        lerp_ticks: Option<i64>,
    ) {
        self.fire_world_border_bounds_changed(level, lerp_ticks);
    }
    /// The server world's border center changed.
    pub(in crate::game) fn native_world_border_center_changed(&mut self, level: LevelRef) {
        self.fire_world_border_center_changed(level);
    }
    /// The server completed a world save cycle. `wait` says chunk writes were
    /// flushed before returning.
    pub(in crate::game) fn native_world_saved(&mut self, wait: bool) {
        self.fire_world_saved(wait);
    }
    /// On a server `RecipesHost::known_recipes` has no client recipe-book UI state.
    pub(in crate::game) fn known_recipes(&mut self) -> Arc<[i32]> {
        Arc::from([])
    }
    /// On a server `RecipesHost::recipe_book_category` has no client recipe-book UI state.
    pub(in crate::game) fn recipe_book_category(
        &mut self,
        _kind: &str,
    ) -> Option<mod_api::item::recipe_book::RecipeBookCategory> {
        None
    }
    /// Whether the owner of this menu accepts button `id`. The active menu's owner
    /// decides its own IDs; the game does not impose a shared numeric range.
    pub(in crate::game) fn native_menu_button_valid(
        &self,
        menu: &str,
        block: Option<&str>,
        id: i32,
        accepted: &mut bool,
    ) {
        self.fire_menu_button_valid(menu, block, id, accepted);
    }
    /// A server broadcast line is about to be logged and sent. Change `line`
    /// or set `cancel` to drop it.
    pub(in crate::game) fn native_broadcast_message(&self, line: &mut Text, cancel: &mut bool) {
        self.fire_broadcast_message(line, cancel);
    }
    /// A hopper is about to move one item between containers, a composter's
    /// bone meal included. Set `cancel` to leave it in its source slot. A chest
    /// boat or minecart is reported at the block it is in.
    pub(in crate::game) fn native_inventory_move_item(
        &self,
        source: [i32; 3],
        destination: [i32; 3],
        item: &str,
        cancel: &mut bool,
    ) {
        self.fire_inventory_move_item(source, destination, item, cancel);
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;
