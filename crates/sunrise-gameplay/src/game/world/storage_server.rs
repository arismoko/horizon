use crate::game::ServerGame;
use sunrise_core::random::RandomSource;
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::chests::facing_step;
use sunrise_gameplay_common::chests::is_shulker_box;
use sunrise_level::containers::{BlockEntity, is_chest_block};

impl ServerGame<'_> {
    /// Closing it copies the items back and keeps the block entity, as vanilla's
    /// `EnderChestBlockEntity` persists across open/close (`close_ender_chest`
    /// must not remove it, or `/data get block` loses it after a use).
    pub fn close_ender_chest(&mut self) {
        let Some(key) = self.server.ender_open.take() else {
            return;
        };
        // The items go back to the player: an `EnderChestBlockEntity` holds none, so
        // nothing of theirs stays in the chunk or in `/data get block`.
        if let Some(BlockEntity::Chest(items)) = self.common.world.block_entities.get_mut(&key) {
            self.server.ender_items = std::mem::replace(items, vec![None; 27]);
        }
    }
    /// Opening an ender chest shows the player's ender inventory in its place.
    pub fn open_ender_chest(&mut self, key: (i32, i32, i32)) {
        self.common
            .world
            .block_entities
            .insert(key, BlockEntity::Chest(self.server.ender_items.clone()));
        self.common.container = Some(key);
        self.server.ender_open = Some(key);
    }
    /// `ServerPlayerGameMode.useItemOn` for a spectator: the block's menu
    /// (`BlockState.getMenuProvider`) opens, and nothing else happens. The
    /// lid, its sound and the `container_open` game event come from
    /// `ContainerOpenersCounter`, which spectators never join
    /// (`ChestBlockEntity.startOpen`); the stat, the piglins' anger and the
    /// shulker box's room check are `useWithoutItem`'s, which is not called.
    /// Only the block containers whose clicks the server takes back from a
    /// spectator (`handleContainerClick`) open here: storage, the furnaces,
    /// the brewing stand and the crafter.
    pub fn spectator_open_menu(&mut self, pos: [i32; 3]) {
        if self.fake_blocks(mod_api::FakeEffect::Menu) {
            return;
        }
        let key = (pos[0], pos[1], pos[2]);
        let name = self
            .common
            .res
            .facts()
            .get(self.common.world.block(pos[0], pos[1], pos[2]))
            .name
            .clone();
        let container = is_shulker_box(&name)
            || matches!(
                name.trim_start_matches("minecraft:"),
                "chest"
                    | "trapped_chest"
                    | "barrel"
                    | "hopper"
                    | "dispenser"
                    | "dropper"
                    | "furnace"
                    | "blast_furnace"
                    | "smoker"
                    | "brewing_stand"
                    | "crafter"
            );
        // ChestBlock.getMenuProvider: none while either half is blocked.
        if !container || self.chest_menu_blocked(key, &name) {
            return;
        }
        // BaseContainerBlockEntity.canOpen: a spectator opens no container that still
        // has a loot table to unpack (it clears while it rolls, on first open).
        // ServerPlayer.openMenu sends the refused spectator a red action bar.
        if self
            .common
            .world
            .chunks
            .get(&(key.0 >> 4, key.2 >> 4))
            .and_then(|c| c.saved.as_ref())
            .is_some_and(|s| s.pending_loot.contains_key(&key))
        {
            self.server_send_overlay_message(
                Text::translate("container.spectatorCantOpen", &[])
                    .with_color(sunrise_game_data::text::RED),
            );
            return;
        }
        let Some(entity) = BlockEntity::for_block(&name) else {
            return;
        };
        self.common
            .world
            .block_entities
            .get_or_insert_with(key, || entity);
        self.common.container = Some(key);
        self.server.open_container_slots = Some((key, self.common.open_container_size()));
        self.server_set_acting_use(false);
    }
    /// `Container.stillValidBlockEntity`: the open block container closes once
    /// the player's eyes are out of reach of any of its blocks (a 4-block buffer).
    pub fn tick_container_reach(&mut self) {
        // A server's menu ends when the server closes it.
        if !self.common.is_logical_server() {
            return;
        }
        let Some(eye) = self.common.eye_position() else {
            return;
        };
        if self
            .common
            .container
            .is_none_or(|k| k.1 == super::minecarts::CART_KEY_Y)
        {
            return;
        }
        // `Container.stillValidBlockEntity`: the menu holds a block entity per
        // part, and a part that went away (`getBlockEntity(pos) != this`) ends
        // it. A double chest whose other half broke would otherwise show a
        // shorter container.
        if self
            .server
            .open_container_slots
            .is_some_and(|(open, slots)| {
                self.common.container == Some(open) && self.common.open_container_size() < slots
            })
        {
            self.close_container_server();
            return;
        }
        let range = self.common.reach() + 4.0;
        let near = self.common.container_parts().iter().all(|&((x, y, z), _)| {
            let within = sunrise_sim::lan::guest::within_block_range(eye, range, [x, y, z]);
            self.host_within_interaction_range([x, y, z], within)
        });
        if !near {
            self.close_container_server();
        }
    }

    /// A menu's `stillValid` reach (`isWithinBlockInteractionRange(pos, 4)`) for the
    /// world's own player: `within` is the game's answer, then a mod's
    /// `block_interaction_range`.
    pub(in crate::game) fn host_within_interaction_range(
        &self,
        pos: [i32; 3],
        within: bool,
    ) -> bool {
        let player = self
            .common
            .level
            .player_uuid
            .map(sunrise_sim::lan::playerdata::ints_uuid);
        self.within_interaction_range_of(player, pos, 4.0, within)
    }
    /// Applies the player's block interaction range hook to the game's answer.
    pub(in crate::game) fn within_interaction_range_of(
        &self,
        player: Option<u128>,
        pos: [i32; 3],
        buffer: f64,
        within: bool,
    ) -> bool {
        let Some(player) = player else {
            return within;
        };
        let mut within = within;
        sunrise_sim::lan::guest::native_block_interaction_range(
            &self.common.loop_context,
            player,
            sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
            buffer,
            &mut within,
        );
        within
    }
    /// `CraftingMenu.stillValid`: the open crafting table closes once its
    /// block is no longer a crafting table or the player's eyes are out of
    /// reach (a 4-block buffer).
    pub(in crate::game) fn tick_craft_table(&mut self) {
        // A server's menu ends when the server closes it.
        if !self.common.is_logical_server() || self.common.craft_size != 3 {
            return;
        }
        let (Some(pos), Some(eye)) = (self.server.craft_table, self.common.eye_position()) else {
            return;
        };
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        let table = &*self.common.res.facts().get(state).name == "minecraft:crafting_table";
        let range = self.common.reach() + 4.0;
        let within = sunrise_sim::lan::guest::within_block_range(eye, range, pos);
        if !(table && self.host_within_interaction_range(pos, within)) {
            self.close_container_server();
        }
    }

    /// `BaseContainerBlockEntity.createMenu` (and `ChestBlock`'s provider for
    /// a double chest, which asks both halves' `canOpen`): a `lock` the opener
    /// cannot unlock refuses the menu with `sendChestLockedNotifications` -
    /// the action bar names the container and one chest-locked sound plays.
    /// True when the open is refused.
    pub fn refuse_locked_container(&mut self, key: (i32, i32, i32), name: &str) -> bool {
        // A carrier transfer fence holds this cell: its block entity's
        // container does not open while the operation runs.
        if self
            .common
            .world
            .carrier_fences
            .locks_cell(&[key.0, key.1, key.2])
        {
            return true;
        }
        let Some(default_key) = sunrise_level::containers::default_name_key(name) else {
            return false;
        };
        // `LockCode.canUnlock` reads `Player.getMainHandItem`, whichever
        // hand the click came from.
        let hand = self.common.stack_in(self.common.selected).cloned();
        let spectator = self.common.spectator();
        // The halves in `DoubleBlockCombiner` order: the right half first.
        let halves = match self.common.chest_partner(key) {
            Some((other, true)) => [Some(key), Some(other)],
            Some((other, false)) => [Some(other), Some(key)],
            None => [Some(key), None],
        };
        if !halves
            .iter()
            .flatten()
            .any(|&half| self.container_locked_for(half, spectator, hand.as_ref()))
        {
            return false;
        }
        let title = match (halves[0], halves[1]) {
            (Some(first), Some(second)) => sunrise_level::containers::double_chest_name(
                self.common.block_name_plain(first).as_deref(),
                self.common.block_name_plain(second).as_deref(),
            ),
            (Some(one), None) => sunrise_level::containers::display_name(
                self.common.block_name_plain(one).as_deref(),
                default_key,
            ),
            _ => sunrise_level::containers::display_name(None, default_key),
        };
        let title = sunrise_game_data::component::name_text(&title);
        self.server_send_overlay_message(Text::translate("container.isLocked", &[title]));
        let at = match (halves[0], halves[1]) {
            (Some(first), Some(second)) => [
                f64::from(first.0 + second.0) / 2.0 + 0.5,
                f64::from(first.1) + 0.5,
                f64::from(first.2 + second.2) / 2.0 + 0.5,
            ],
            (Some(one), None) => [
                f64::from(one.0) + 0.5,
                f64::from(one.1) + 0.5,
                f64::from(one.2) + 0.5,
            ],
            _ => [0.0; 3],
        };
        self.server_play_sound("minecraft:block.chest.locked", at, 1.0, 1.0);
        true
    }

    /// The block above `key` is a redstone conductor.
    pub(in crate::game::server_side) fn conductor_above(&self, key: (i32, i32, i32)) -> bool {
        let above = self.common.world.block(key.0, key.1 + 1, key.2);
        self.common.res.behaviours().facts(above).flags
            & sunrise_world::level::data::state_flags::CONDUCTOR
            != 0
    }

    /// `ChestBlock.isChestBlockedAt`: a redstone conductor above the chest or
    /// a cat sitting on it.
    pub(in crate::game::server_side) fn chest_blocked_at(&self, key: (i32, i32, i32)) -> bool {
        self.conductor_above(key) || super::machines::cat_sitting_above(&self.common.mobs, key)
    }

    /// `ChestBlock.getMenuProvider` (`combine(.., false)`) and
    /// `EnderChestBlock.useWithoutItem`: a blocked chest, or a double chest
    /// with either half blocked, opens no menu.
    pub fn chest_menu_blocked(&self, key: (i32, i32, i32), name: &str) -> bool {
        match name {
            n if is_chest_block(n) => {
                self.chest_blocked_at(key)
                    || self
                        .common
                        .chest_partner(key)
                        .is_some_and(|(other, _)| self.chest_blocked_at(other))
            }
            "minecraft:ender_chest" => self.conductor_above(key),
            _ => false,
        }
    }

    /// `ChestBlockEntity.playSound`: resolve the sound for the first or last opener.
    pub(in crate::game) fn resolve_container_sound(
        &self,
        key: (i32, i32, i32),
        name: &str,
        open: bool,
    ) -> Option<(&'static str, [f64; 3])> {
        let chest = is_chest_block(name);
        let sound = if is_shulker_box(name) {
            if open {
                "minecraft:block.shulker_box.open"
            } else {
                "minecraft:block.shulker_box.close"
            }
        } else if name == "minecraft:ender_chest" {
            if open {
                "minecraft:block.ender_chest.open"
            } else {
                "minecraft:block.ender_chest.close"
            }
        } else if chest {
            sunrise_sim::mobs::transport_items::chest_hinge_sound(name, open)
        } else if name == "minecraft:barrel" {
            if open {
                "minecraft:block.barrel.open"
            } else {
                "minecraft:block.barrel.close"
            }
        } else {
            return None;
        };
        let mut at = [
            f64::from(key.0) + 0.5,
            f64::from(key.1) + 0.5,
            f64::from(key.2) + 0.5,
        ];
        if chest && let Some((other, _)) = self.common.chest_partner(key) {
            at[0] = f64::from(key.0 + other.0) / 2.0 + 0.5;
            at[2] = f64::from(key.2 + other.2) / 2.0 + 0.5;
        } else if name == "minecraft:barrel"
            && let Some(state) = self
                .common
                .pack
                .assets
                .registry
                .state(self.common.world.block(key.0, key.1, key.2))
            && let Some(facing) = state.properties.get("facing")
        {
            let (dx, dy, dz) = match facing.as_str() {
                "up" => (0.0, 0.5, 0.0),
                "down" => (0.0, -0.5, 0.0),
                f => {
                    let (x, z) = facing_step(f);
                    (f64::from(x) / 2.0, 0.0, f64::from(z) / 2.0)
                }
            };
            at = [at[0] + dx, at[1] + dy, at[2] + dz];
        }
        Some((sound, at))
    }

    pub(in crate::game) fn play_container_sound(&mut self, sound: &str, at: [f64; 3]) {
        let pitch = self.common.world.random.next_float() * 0.1 + 0.9;
        self.server_play_sound(sound, at, 0.5, pitch);
    }

    /// Play the current block's sound when its opener count crosses zero
    /// (`ContainerOpenersCounter.onOpen` / `onClose`).
    pub fn container_edge_sound(&mut self, key: (i32, i32, i32), open: bool) {
        let state = self.common.world.block(key.0, key.1, key.2);
        let name = &self.common.res.facts().get(state).name;
        if let Some((sound, at)) = self.resolve_container_sound(key, name, open) {
            self.play_container_sound(sound, at);
        }
    }
}
