use super::*;
use sunrise_gameplay_common::game::native_hooks;

impl crate::game::ServerGame<'_> {
    /// Asks the mods whether the acting guest may empty `stack` at `pos`
    /// (`native_bucket_empty`): true when one cancelled it. A game's own
    /// player is not asked.
    pub(in crate::game) fn bucket_empty_cancelled(
        &mut self,
        stack: &ItemStack,
        pos: [i32; 3],
    ) -> bool {
        let (Some(uuid), Some(name)) = (
            self.server.lan_acting_uuid,
            self.server.lan_acting_name.clone(),
        ) else {
            return false;
        };
        let mod_fluid = sunrise_world::fluid::mod_fluid_of_bucket(&stack.item);
        let fluid = match &*stack.item {
            "minecraft:lava_bucket" => "minecraft:lava",
            "minecraft:powder_snow_bucket" => "minecraft:powder_snow",
            _ => mod_fluid.map_or("minecraft:water", |fluid| fluid.get().id.as_str()),
        };
        let mut cancel = false;
        self.native_bucket_empty(uuid, &name, pos, fluid, &mut cancel);
        cancel
    }
    /// `Player.interactOn` with the acting hand: whether it took the click. A
    /// guest's interact packet names its entity (`target`), which the helpers
    /// use as it is; the local player picks from the crosshair instead, the
    /// way `Minecraft.hitResult` does. `native_entity_interact_specific` and
    /// `native_interact_entity` hear it first; a claim swings the hand and
    /// ends the click.
    pub fn interact_entity(&mut self, target: Option<InteractTarget>) -> Result<bool> {
        // Native mods, where Player.interactOn starts: before the entity's
        // own interact and the item's interactLivingEntity. The entity is the
        // one the packet named, or the one the crosshair picked.
        let (named, distance) = match target {
            Some(t) => (Some(t.id), None),
            None => match self.server_crosshair_entity_hit() {
                Some(hit) => (Some(hit.id), Some(hit.distance)),
                None => (None, None),
            },
        };
        // The mod hooks hear no natural fence knot (#13036): its click
        // keeps the knot's own path below.
        if let Some(id) = named
            && let Some(entity) = self
                .level_entity(id)
                .filter(|t| !matches!(t, commands::Target::Knot(_)))
        {
            // The point clicked, measured from the entity's position: a
            // guest's packet carries it, and the local player's is where the
            // crosshair's ray met the entity (`EntityHitResult.getLocation()`).
            let location = match target {
                Some(t) => Some(t.location),
                None => distance
                    .zip(self.aim())
                    .zip(self.target_position(entity))
                    .map(|((d, (eye, dir)), at)| {
                        std::array::from_fn(|k| eye[k] + dir[k] * d - at[k])
                    }),
            };
            let kind = self.target_type(entity);
            let stack = self.common.held().cloned();
            let mut claimed = false;
            if let Some(location) = location {
                self.native_entity_interact_specific(
                    id,
                    &kind,
                    stack.as_ref(),
                    location,
                    &mut claimed,
                );
            }
            if claimed {
                self.swing_arm();
                return Ok(true);
            }
            let mut result = None;
            self.native_entity_interact(self.common.hand, id, &kind, stack.as_ref(), &mut result);
            if let Some(answer) = result
                && !matches!(
                    answer,
                    mod_api::InteractionResult::Pass | mod_api::InteractionResult::TryWithEmptyHand
                )
            {
                return Ok(self.interaction_consumed(answer));
            }
            if matches!(entity, commands::Target::General(_)) {
                // A plain Entity has no default interaction. A pass stays on this row.
                return Ok(false);
            }
        }
        // `AbstractNautilus.interact` keeps a clicked nautilus
        // (`setPersistenceRequired`), used or not, before its own handling runs.
        self.nautilus_clicked(named);
        Ok(self.interact_frame(target)
            || self.interact_armor_stand(target)
            || self.interact_cushion(target)
            || self.interact_boat(target)
            || self.interact_cart(target)
            || self.interact_mob(target)?
            || self.item_interact_mob(target))
    }
    /// Whether `now` is still the stack `before` was: the same item and the
    /// same count, or both empty. The check after a hook that may change the
    /// world is this shallow on purpose; components are not compared.
    pub(in crate::game) fn same_item_and_count(
        now: Option<&ItemStack>,
        before: Option<&ItemStack>,
    ) -> bool {
        match (now, before) {
            (None, None) => true,
            (Some(a), Some(b)) => a.item == b.item && a.count == b.count,
            _ => false,
        }
    }
    /// The player as a block's `useItemOn` / `useWithoutItem` sees them.
    pub fn use_context(&self) -> PlayerContext {
        PlayerContext {
            yaw: self.common.yaw,
            pitch: self.common.pitch,
            may_build: self.may_build(),
            can_eat: self.common.creative
                || self
                    .common
                    .player
                    .as_ref()
                    .is_some_and(|p| p.health.food.food < 20),
            game_master: self.common.can_use_game_master_blocks(),
            // RespawnConfig.isSamePosition compares the dimension and the block;
            // where an anchor cannot set spawn (`canSetSpawn`), a click on it
            // explodes before that comparison.
            respawn_point: self
                .server
                .respawn_config
                .filter(|c| self.anchor_works() && c.dim == self.common.dim)
                .map(|c| BlockPos::new(c.pos[0], c.pos[1], c.pos[2])),
        }
    }
    /// The hand whose use ended the click, if any.
    pub fn use_item(&mut self) -> Result<Option<Hand>> {
        self.with_player_actor(|game| game.use_item_as_player())
    }
    /// `Minecraft.startUseItem`: the main hand, then the off hand.
    pub fn use_item_as_player(&mut self) -> Result<Option<Hand>> {
        // Spectators only look (MultiPlayerGameMode.useItemOn/useItem pass),
        // but on a server a spectator's use on a block opens the block's
        // menu (ServerPlayerGameMode.useItemOn).
        if self.common.spectator() {
            if let Some(GuestUse::OnBlock { hand, .. }) = self.server.guest_use
                && let Some(hit) = self.pick()
            {
                self.with_hand(hand, |game| {
                    let held = game.held().cloned();
                    if matches!(
                        game.right_click_block(&hit, hand, held.as_ref()),
                        native_hooks::BlockClickAnswer::Go { .. }
                    ) {
                        game.spectator_open_menu(hit.pos);
                    }
                });
            }
            return Ok(None);
        }
        // A carrot or warped fungus on a stick boosts the ridden pig or strider.
        // A guest's use packets name one hand and touch no entity: the client
        // sends one per hand and part of its click, and Interact for entities.
        let guest = self.server.guest_use;
        if let Some(GuestUse::Entity(hand, target)) = guest {
            return Ok(self
                .with_hand(hand, |g| g.interact_entity(Some(target)))?
                .then_some(hand));
        }
        let on_block = matches!(guest, Some(GuestUse::OnBlock { .. }));
        if !on_block
            && self.common.riding.is_some()
            && let Some(item) = self.common.held().map(|s| s.item.clone())
            && self.use_steering_stick(&item)
        {
            return Ok(Some(Hand::Main));
        }
        if guest.is_none() && self.interact_entity(None)? {
            return Ok(Some(Hand::Main));
        }
        // An EntityHitResult may fall through to Item.use, never to useItemOn.
        let entity = guest.is_none() && self.server_crosshair_entity_hit().is_some();
        let block_hit = match guest {
            Some(GuestUse::OnBlock { .. }) => self.pick(),
            Some(GuestUse::Item(_)) => None,
            _ if entity => None,
            _ => self.pick(),
        };
        let hands = match guest {
            Some(
                GuestUse::OnBlock { hand, .. } | GuestUse::Item(hand) | GuestUse::Entity(hand, _),
            ) => {
                vec![hand]
            }
            None => Hand::BOTH.to_vec(),
        };
        for hand in hands {
            // The off hand's turn starts on the entity too (Minecraft.startUseItem).
            if entity && hand == Hand::Off && self.with_hand(hand, |g| g.interact_entity(None))? {
                return Ok(Some(hand));
            }
            if self.with_hand(hand, |game| game.use_hand(block_hit.as_ref()))? {
                return Ok(Some(hand));
            }
        }
        Ok(None)
    }
    pub(in crate::game) fn server_consume_held_slot(&mut self, amount: u32) {
        self.edit_block_use_stack(|cell| {
            if let Some(stack) = cell.as_mut() {
                stack.count = stack.count.saturating_sub(amount);
                if stack.count == 0 {
                    *cell = None;
                }
            }
        });
    }
    pub(in crate::game) fn server_apply_use_cooldown(&mut self, stack: &ItemStack) {
        self.apply_use_cooldown(stack);
    }
    pub(in crate::game) fn server_acting_player_ref(&self) -> Option<mod_api::PlayerRef> {
        let uuid = self.server.lan_acting_uuid.or_else(|| {
            self.common
                .level
                .player_uuid
                .map(sunrise_sim::lan::playerdata::ints_uuid)
        })?;
        self.player(uuid)
    }
    pub(in crate::game) fn server_current_level_ref(&self) -> Option<mod_api::LevelRef> {
        self.level(self.common.dim.id())
    }
}

use sunrise_gameplay_common::game::native_hooks::base::GameHooks as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::use_item_hand::UseItemHandGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
