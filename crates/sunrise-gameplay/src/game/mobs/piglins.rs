//! The game side of piglins (`PiglinAi`): spotting and picking up gold,
//! bartering when the admiring is over, and anger at players who open
//! their chests or mine their gold.
use crate::game::ServerGame;
use sunrise_core::{geometry::Vec3, random::RandomSource};
use {
    sunrise_items::items::ItemStack, sunrise_sim::first_person_sim::SwingAnimation,
    sunrise_sim::mobs::Mob, sunrise_sim::mobs::Species, sunrise_sim::mobs::equip,
    sunrise_sim::mobs::equip::Slot, sunrise_sim::mobs::finalize::OFFHAND,
    sunrise_sim::mobs::finalize::SLOT_NAMES, sunrise_sim::mobs::finalize::item_stack,
    sunrise_sim::mobs::finalize::set_drop_chance, sunrise_sim::mobs::finalize::set_item_stack,
    sunrise_sim::mobs::finalize::set_slot_enchantments,
    sunrise_sim::mobs::finalize::stored_drop_chance, sunrise_sim::mobs::piglin::ADMIRE,
    sunrise_sim::mobs::piglin::eat, sunrise_sim::mobs::piglin::has_eaten_recently,
    sunrise_sim::mobs::piglin::is_idle,
};

impl ServerGame<'_> {
    /// `ZombifiedPiglin.alertOthers`: keep the source's player binding and
    /// refuse recipients that already have a target or are allied to it.
    pub(in crate::game::server_side) fn piglin_alert_target(
        &mut self,
        alert: sunrise_sim::mobs::undead::PiglinAlert,
    ) {
        use sunrise_sim::mobs::Who;
        let sunrise_sim::mobs::undead::PiglinAlert {
            mob,
            source,
            target,
        } = alert;
        let player = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.mob_players.get(&source).copied())
            .or(self.server.lan_acting);
        let target_team = match target {
            Who::Mob(id) => self.mob_team(id),
            Who::Player => match player {
                Some(conn) => self
                    .command_guest(conn)
                    .and_then(|guest| self.common.score.board.players_team(&guest.name)),
                None => self.player_team(),
            },
        };
        if target == Who::Mob(mob)
            || sunrise_game_data::scoreboard::teams_allied(self.mob_team(mob), target_team)
            || self
                .common
                .mobs
                .iter()
                .find(|other| other.id == mob)
                .is_none_or(|other| other.ai.target.is_some())
        {
            return;
        }
        self.set_event_ai_target(mob, Some(target), player);
    }

    /// `ItemStack.has(DataComponents.BLOCKS_ATTACKS)` for an item id.
    fn item_blocks_attacks(&self, item: &str) -> bool {
        self.common
            .res
            .items()
            .get(item)
            .is_some_and(|i| i.blocks_attacks.is_some())
    }
    /// `PiglinAi.isLovedItem` or a preferred weapon (`Piglin.canReplaceCurrentItem`).
    fn piglin_wanted(&self, m: &Mob, item: &str) -> bool {
        self.piglin_loved(item)
            || !m.is_baby() && self.item_in_tag("minecraft:piglin_preferred_weapons", item)
    }
    /// The `SLOT_NAMES` index `getEquipmentSlotForItem` picks: the
    /// `equippable` slot (armour, a shield's off hand), else the main hand.
    fn piglin_slot(&self, item: &str) -> usize {
        let slot = self
            .common
            .res
            .items()
            .get(item)
            .and_then(|i| i.equip_slot.as_deref())
            .filter(|s| matches!(*s, "head" | "chest" | "legs" | "feet" | "offhand"))
            .unwrap_or("mainhand");
        SLOT_NAMES.iter().position(|n| *n == slot).unwrap_or(0)
    }
    /// `Piglin.canReplaceCurrentItem`: an empty slot takes anything, a
    /// `PREVENT_ARMOR_CHANGE` piece stays, a wanted item replaces an unwanted
    /// one and never the other way round; when both or neither are wanted,
    /// `Mob.canReplaceCurrentItem` compares armour or weapons.
    fn piglin_can_replace(&self, m: &Mob, stack: &ItemStack, slot: usize) -> bool {
        let Some(current) = item_stack(m, slot) else {
            return true;
        };
        let new_wanted = self.piglin_wanted(m, &stack.item);
        let current_wanted = self.piglin_wanted(m, &current.item);
        let slot = [
            Slot::MainHand,
            Slot::OffHand,
            Slot::Feet,
            Slot::Legs,
            Slot::Chest,
            Slot::Head,
        ]
        .get(slot)
        .copied()
        .unwrap_or(Slot::MainHand);
        let new = self.stack_facts(m, stack, slot);
        let worn = self.stack_facts(m, &current, slot);
        // `getPreferredWeaponType` is the preferred-weapons tag for adults.
        let preferred = (!m.is_baby()).then(|| {
            (
                self.item_in_tag("minecraft:piglin_preferred_weapons", &stack.item),
                self.item_in_tag("minecraft:piglin_preferred_weapons", &current.item),
            )
        });
        piglin_replace(new_wanted, current_wanted, &new, &worn, slot, preferred)
    }
    /// `SimpleContainer.canAddItem` on the piglin's eight slots.
    fn piglin_can_add(&self, m: &Mob, stack: &ItemStack) -> bool {
        m.extra.inventory.len() < 8
            || m.extra.inventory.iter().any(|s| {
                s.same_item_same_components(stack) && s.count < self.piglin_max_stack(&s.item)
            })
    }
    fn piglin_max_stack(&self, item: &str) -> u32 {
        self.items().get(item).map_or(64, |i| i.max_stack)
    }
    /// `PiglinAi.wantsToPickup`: babies ignore their own item tag, but can
    /// pick up currency and other loved items just like adults.
    fn piglin_wants(&self, m: &Mob, stack: &ItemStack) -> bool {
        let item = &*stack.item;
        if m.is_baby() && self.item_in_tag("minecraft:ignored_by_piglin_babies", item)
            || self.item_in_tag("minecraft:piglin_repellents", item)
        {
            return false;
        }
        if m.extra.piglin.admire_disabled.present() && m.ai.target.is_some() {
            return false;
        }
        let holds_loved = m.extra.off_hand_id().is_some_and(|o| self.piglin_loved(o));
        if self.piglin_currency(stack) {
            return !holds_loved;
        }
        let space = self.piglin_can_add(m, stack);
        if item == "minecraft:gold_nugget" {
            space
        } else if self.item_in_tag("minecraft:piglin_food", item) {
            !has_eaten_recently(m) && space
        } else if self.piglin_loved(item) {
            !holds_loved && space
        } else {
            self.piglin_can_replace(m, stack, self.piglin_slot(item))
        }
    }
    /// `Mob.equipItemIfPossible` for a piglin: put it in its slot when that
    /// slot takes it (an armour piece that cannot replace what is worn goes
    /// to an empty main hand), dropping what it replaces by drop chance.
    fn piglin_equip(&mut self, index: usize, stack: &ItemStack) -> bool {
        let item = &*stack.item;
        let mut slot = self.piglin_slot(item);
        let m = &self.common.mobs[index];
        let mut replace = self.piglin_can_replace(m, stack, slot);
        if slot > 1 && !replace {
            slot = 0;
            replace = m.extra.main_hand_id().is_none();
        }
        if !replace {
            return false;
        }
        let m = &mut self.common.mobs[index];
        let old = item_stack(m, slot);
        let chance = stored_drop_chance(m, slot);
        let drop_old = old.is_some() && (m.random.next_float() - 0.1).max(0.0) < chance;
        // The piglin keeps the stack whole: its components, enchantments and trim.
        set_item_stack(
            m,
            slot,
            ItemStack {
                count: 1,
                ..stack.clone()
            },
        );
        // setItemSlotAndDropWhenKilled, then persistenceRequired.
        set_drop_chance(m, slot, 2.0);
        m.persistent = true;
        let at = [m.position.x, m.position.y, m.position.z];
        if let (true, Some(old)) = (drop_old, old) {
            self.spawn_item_own(old, at, 10);
        }
        true
    }
    /// `PiglinAi.putInInventory`: what does not fit is thrown at a random
    /// spot nearby.
    fn piglin_stash(&mut self, index: usize, mut stack: ItemStack) {
        let max = self.piglin_max_stack(&stack.item);
        let m = &mut self.common.mobs[index];
        for s in &mut m.extra.inventory {
            if stack.count > 0 && s.same_item_same_components(&stack) && s.count < max {
                let n = (max - s.count).min(stack.count);
                s.count += n;
                stack.count -= n;
            }
        }
        while stack.count > 0 && m.extra.inventory.len() < 8 {
            let n = stack.count.min(max);
            m.extra.inventory.push(ItemStack {
                count: n,
                ..stack.clone()
            });
            stack.count -= n;
        }
        if stack.count == 0 {
            return;
        }
        let r = &mut self.common.world.random;
        let to = [
            m.position.x + f64::from(r.next_float() * 8.0 - 4.0),
            m.position.y + 1.0,
            m.position.z + f64::from(r.next_float() * 8.0 - 4.0),
        ];
        self.piglin_throw(index, to, stack);
    }
    /// `PiglinAi.stopHoldingOffHandItem` for something that is not
    /// bartering currency: wear it if it can, else keep it in the inventory.
    fn piglin_keep(&mut self, index: usize, stack: ItemStack) {
        // Nuggets keep their whole stack in the inventory (a hand slot here
        // holds one item id only).
        if &*stack.item == "minecraft:gold_nugget" || !self.piglin_equip(index, &stack) {
            self.piglin_stash(index, stack);
        }
    }
    /// `BehaviorUtils.throwItem`: toss a stack from a piglin toward `target_pos`.
    /// `targetPos` is elevated by 1.0 block per `throwItemsTowardPos`.
    /// Throw vector is `(targetPos - thrower.position()).normalize() * 0.30000001192092896`.
    /// Pickup delay is default (10 ticks). Hand Y pos is `eyeY - 0.3`.
    fn piglin_throw(&mut self, index: usize, target_pos: [f64; 3], stack: ItemStack) {
        let m = &self.common.mobs[index];
        let from = [m.position.x, m.eye_y() - 0.3, m.position.z];
        let d = [
            target_pos[0] - m.position.x,
            target_pos[1] - m.position.y,
            target_pos[2] - m.position.z,
        ];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
        const VELOCITY: f64 = 0.30000001192092896;
        let v = [
            d[0] / len * VELOCITY,
            d[1] / len * VELOCITY,
            d[2] / len * VELOCITY,
        ];
        self.spawn_item(stack, from, v, 10);
    }
    /// `PiglinAi.isWearingSafeArmor` for the acting player: a stack in an armour
    /// slot (feet, legs, chest, head, `EquipmentSlotGroup.ARMOR`'s order) in
    /// `#minecraft:piglin_safe_armor`, once the parts of its item answered
    /// (`ItemPart::makes_piglins_neutral`). An item with no part for it costs one
    /// table lookup.
    pub(in crate::game::server_side) fn wearing_gold(&self) -> bool {
        let tag = self
            .common
            .res
            .items()
            .item_tags
            .get("minecraft:piglin_safe_armor");
        self.common.armor.iter().rev().flatten().any(|s| {
            let mut neutral = tag.is_some_and(|t| t.contains(&*s.item));
            if mod_api::__private::item_listens(mod_api::HookId::MakesPiglinsNeutral, &s.item)
                && let Some(epoch) = self.common.native_lifecycle.epoch()
                && let Some(p) = self.common.player.as_ref()
            {
                let wearer =
                    mod_api::LivingRef::new(mod_api::EntityRef::new(epoch, p.entity_id as u32));
                self.native_makes_piglins_neutral(s, wearer, &mut neutral);
            }
            neutral
        })
    }
    /// Piglins notice wanted items on the ground within 9 blocks (`PiglinAi.wantsToPickup`:
    /// gold and other loved items, nuggets, food, gear), walk to them and
    /// take them (`GoToWantedItem`, `Piglin.pickUpItem`).
    pub fn piglin_pickups(&mut self) {
        let griefing = self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::MOB_GRIEFING);
        if !griefing && !self.grief_hooked() {
            return;
        }
        for i in 0..self.common.mobs.len() {
            let m = &self.common.mobs[i];
            if m.species() != Species::Piglin
                || !m.alive()
                || !self.common.creatures.ticked.contains(&m.id)
            {
                continue;
            }
            // `Piglin.wantsToPickUp`: NeoForge's `canEntityGrief` for the piglin.
            if !self.entity_grief(m.id, griefing) {
                continue;
            }
            let pos = m.position;
            // NearestItemSensor: the nearest wanted item within 32 that the piglin
            // sees (eye to eye); GoToWantedItem then walks to it within 9 blocks.
            let w = self.mob_world_base(&[]);
            let eye = Vec3::new(pos.x, m.eye_y(), pos.z);
            let nearest = self
                .common
                .dropped
                .iter()
                .enumerate()
                .map(|(j, e)| (j, e.position.distance_squared(pos)))
                .filter(|(_, d)| *d < 32.0 * 32.0)
                .filter(|&(j, _)| self.piglin_wants(m, &self.common.dropped[j].stack))
                .filter(|&(j, _)| {
                    // ItemEntity's eye height is 0.2125.
                    let e = &self.common.dropped[j];
                    let at = Vec3::new(e.position.x, e.position.y + 0.2125, e.position.z);
                    w.line_of_sight(eye, at)
                })
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let loved =
                nearest.is_some_and(|(j, _)| self.piglin_loved(&self.common.dropped[j].stack.item));
            self.admire_seen_item(i, nearest.map(|(_, d)| d), loved);
            let best = nearest.filter(|(_, d)| *d <= 81.0);
            let Some((j, _)) = best else {
                self.common.mobs[i].extra.point = None;
                continue;
            };
            let e = &self.common.dropped[j];
            let close = (e.position.x - pos.x).abs() <= 1.3
                && (e.position.z - pos.z).abs() <= 1.3
                && (e.position.y - pos.y).abs() <= 1.0;
            if !close || e.pickup_delay > 0 {
                self.common.mobs[i].extra.point = Some([e.position.x, e.position.y, e.position.z]);
                continue;
            }
            // Nuggets come whole, anything else one at a time.
            let (taken_item, id) = (e.clone(), self.common.mobs[i].id);
            let item = e.stack.item.clone();
            self.server_mob_take_item(&taken_item, id);
            // LivingEntity.onItemPickup, before PiglinAi.pickUpItem splits the stack.
            self.mob_item_pickup_trigger(i, &taken_item);
            let stack = if &*item == "minecraft:gold_nugget" {
                self.remove_dropped(j).stack
            } else {
                let e = &mut self.common.dropped[j];
                e.stack.count -= 1;
                let one = ItemStack {
                    count: 1,
                    ..e.stack.clone()
                };
                if e.stack.count == 0 {
                    self.remove_dropped(j);
                }
                one
            };
            if self.piglin_loved(&item) {
                self.start_admiring(i, stack);
            } else if self.item_in_tag("minecraft:piglin_food", &item)
                && !has_eaten_recently(&self.common.mobs[i])
            {
                // PiglinAi.eat: the food is gone and it will not eat for a while.
                eat(&mut self.common.mobs[i]);
                self.common.mobs[i].nav_stop();
            } else {
                self.common.mobs[i].nav_stop();
                self.piglin_keep(i, stack);
            }
        }
    }
    /// `StartAdmiringItemIfSeen` and the admire activity's stop behaviours, from the
    /// distance squared to `NEAREST_VISIBLE_WANTED_ITEM` and whether that item is loved.
    fn admire_seen_item(&mut self, index: usize, nearest: Option<f64>, loved: bool) {
        let m = &mut self.common.mobs[index];
        let holding = m.extra.off_hand_id().is_some();
        // ADMIRING_ITEM remains present while the off hand is admired.
        if holding && m.extra.using_item > 0 {
            return;
        }
        m.extra.piglin.admire_seen(holding, nearest, loved);
    }
    /// `PiglinAi.holdInOffhand` and `admireGoldItem`: the piglin holds `stack`, the one item it
    /// took (`split(1)` from the ground, `consumeAndReturn(1)` from a hand), with its components.
    fn start_admiring(&mut self, index: usize, stack: ItemStack) {
        // PiglinAi.holdInOffhand: what the off hand held is dropped, whole.
        if let Some(old) = item_stack(&self.common.mobs[index], OFFHAND) {
            let m = &mut self.common.mobs[index];
            m.extra
                .set_slot_stack(sunrise_items::equipment::OFFHAND, None);
            set_slot_enchantments(m, OFFHAND, Vec::new());
            let at = [m.position.x, m.position.y, m.position.z];
            self.spawn_item_own(old, at, 10);
        }
        let m = &mut self.common.mobs[index];
        // The memory is set again, for the item it holds now.
        m.extra.piglin.seen_admire.erase();
        sunrise_sim::mobs::piglin::hold_in_offhand(m, stack);
        // `admireGoldItem`: `ADMIRING_ITEM` for `ADMIRE` ticks, kept as `Ttl` keeps a
        // memory (the brain counts it down before it reads it).
        m.extra.using_item = ADMIRE + 1;
        m.extra.point = None;
        m.nav_stop();
        // The admiring grunt is `PiglinAi.updateActivity`'s: `piglin::brain_step`
        // plays it when the activity changes.
    }
    /// `PiglinAi.mobInteract`: hand a piglin the held `stack` (`canAdmire`: only
    /// piglin currency is accepted from the hand; other loved items are passed over).
    pub fn give_piglin(&mut self, index: usize, stack: &ItemStack) -> bool {
        let m = &self.common.mobs[index];
        if m.species() != Species::Piglin
            || m.is_baby()
            || m.extra.using_item > 0
            || m.extra.piglin.seen_admire.present()
            || m.extra.piglin.admire_disabled.present()
            || !self.piglin_currency(stack)
        {
            return false;
        }
        self.start_admiring(
            index,
            ItemStack {
                count: 1,
                ..stack.clone()
            },
        );
        true
    }
    /// `PiglinAi.stopHoldingOffHandItem`, when the admiring ran out or a hit
    /// stopped it (`piglin::stop_admiring`, which bars the barter). An adult
    /// barters currency after admiring it, loses it to a hit, and equips or keeps
    /// anything else. Babies equip the admired item, or hold it in the main hand
    /// after stashing or throwing the old item. An empty off hand (a second stop
    /// of the same hit) does nothing.
    pub fn finish_admiring(&mut self, mob: u32) {
        let Some(i) = self.common.mobs.iter().position(|m| m.id == mob) else {
            return;
        };
        let barter = !std::mem::take(&mut self.common.mobs[i].extra.piglin.hit_stop);
        if let Some(item) = self.common.mobs[i].extra.off_hand_id()
            && self.item_blocks_attacks(item)
        {
            return;
        }
        let Some(stack) = item_stack(&self.common.mobs[i], OFFHAND) else {
            return;
        };
        let m = &mut self.common.mobs[i];
        m.extra
            .set_slot_stack(sunrise_items::equipment::OFFHAND, None);
        set_slot_enchantments(m, OFFHAND, Vec::new());
        if self.common.mobs[i].is_baby() {
            if !self.piglin_equip(i, &stack) {
                if let Some(old) = item_stack(&self.common.mobs[i], 0) {
                    if self.piglin_loved(&old.item) {
                        self.piglin_stash(i, old);
                    } else {
                        self.piglin_throw_items(i, vec![old]);
                    }
                }
                let m = &mut self.common.mobs[i];
                set_item_stack(m, 0, stack);
                set_drop_chance(m, 0, 2.0);
                m.persistent = true;
            }
            return;
        }
        if !self.piglin_currency(&stack) {
            self.piglin_keep(i, stack);
            return;
        }
        if !barter {
            return;
        }
        // `LootContextParamSets.PIGLIN_BARTER`: the piglin is THIS_ENTITY, and
        // nothing else is supplied.
        let piglin = sunrise_sim::mobs::drops::loot_entity_with_variants(
            &self.common.mobs[i],
            None,
            Some(self.world_variants()),
        );
        let items = self
            .roll_table_at(
                "minecraft:gameplay/piglin_bartering",
                None,
                None,
                Some(&piglin),
            )
            .unwrap_or_default();
        self.piglin_throw_items(i, items);
    }
    /// `PiglinAi.throwItems`: barters and a baby's unwanted main-hand item
    /// use the same player-directed throw.
    pub fn piglin_throw_items(&mut self, i: usize, items: Vec<ItemStack>) {
        if items.is_empty() {
            return;
        }
        // `PiglinAi.throwItemsTowardPos` swings the off hand once before the
        // throws, so the barters leave the hand the piglin admires them in.
        self.common.mobs[i].swing_hand_modified(true, SwingAnimation::DEFAULT);
        let m = &self.common.mobs[i];
        let at = m.position;
        let host = self
            .common
            .player
            .as_ref()
            .filter(|p| !p.health.dead_or_dying() && !self.common.spectator())
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        let guests = self.server.lan.iter().flat_map(|h| {
            h.hub
                .guests
                .iter()
                .filter(|g| h.hub.here(g) && g.loaded && !g.dead && !g.spectator())
                .map(|g| g.position)
        });
        let d = |p: &[f64; 3]| at.distance_squared(Vec3::new(p[0], p[1], p[2]));
        let target = host
            .into_iter()
            .chain(guests)
            .filter(|p| d(p) < 256.0)
            .min_by(|a, b| d(a).total_cmp(&d(b)))
            .map(|p| [p[0], p[1] + 1.0, p[2]]);
        for stack in items {
            let to = target.unwrap_or_else(|| {
                let r = &mut self.common.world.random;
                [
                    at.x + f64::from(r.next_float() * 8.0 - 4.0),
                    at.y + 1.0,
                    at.z + f64::from(r.next_float() * 8.0 - 4.0),
                ]
            });
            self.piglin_throw(i, to, stack);
        }
    }
    /// `PiglinAi.setAngerTargetToNearestTargetablePlayerIfFound`: the nearest
    /// visible targetable player when UNIVERSAL_ANGER is on.
    fn nearest_targetable_player_for_piglin(&self, m: &Mob) -> Option<Option<[i32; 4]>> {
        let range = m.attrs.follow_range;
        let mut best: Option<(Option<[i32; 4]>, f64)> = None;
        if let Some(p) = self.common.player.as_ref()
            && !self.common.creative
            && !self.common.spectator()
            && !p.health.dead_or_dying()
        {
            let eye = [p.position.x, p.position.y + 1.62, p.position.z];
            let d2 = m.distance_sqr(p.position);
            if d2 <= range * range && self.can_see(m.center(), eye) {
                let uuid = self
                    .server
                    .lan_acting_uuid
                    .map(sunrise_sim::lan::playerdata::uuid_ints)
                    .or(self.common.level.player_uuid);
                best = Some((uuid, d2));
            }
        }
        if let Some(h) = self.server.lan.as_ref() {
            for g in &h.hub.guests {
                if h.hub.here(g) && g.loaded && !g.dead && !g.spectator() && g.game_mode != 1 {
                    let eye = [g.position[0], g.position[1] + 1.62, g.position[2]];
                    let pos = Vec3::new(g.position[0], g.position[1], g.position[2]);
                    let d2 = m.distance_sqr(pos);
                    if d2 <= range * range
                        && self.can_see(m.center(), eye)
                        && best.as_ref().is_none_or(|(_, bd)| d2 < *bd)
                    {
                        let uuid = sunrise_sim::lan::playerdata::uuid_ints(g.uuid);
                        best = Some((Some(uuid), d2));
                    }
                }
            }
        }
        best.map(|(uuid, _)| uuid)
    }
    /// `PiglinAi.angerNearbyPiglins`.
    pub(in crate::game) fn anger_piglins(&mut self, only_if_seen: bool) {
        // Fake players never become a mob's target.
        if self.server.lan_acting_fake {
            return;
        }
        let Some(p) = self.common.player.as_ref() else {
            return;
        };
        if self.common.creative || self.common.spectator() {
            return;
        }
        let at = p.position;
        let eye = [at.x, at.y + 1.62, at.z];
        let player_uuid = self
            .server
            .lan_acting_uuid
            .map(sunrise_sim::lan::playerdata::uuid_ints)
            .or(self.common.level.player_uuid);
        let universal = self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::UNIVERSAL_ANGER);
        // The target hook may remove mobs, so the list's end is read at each step.
        let mut next = 0;
        while next < self.common.mobs.len() {
            let i = next;
            next += 1;
            let m = &self.common.mobs[i];
            if m.species() != Species::Piglin
                || m.is_baby()
                || !m.alive()
                || !is_idle(m)
                || (m.position.x - at.x).abs() > 16.0
                || (m.position.y - at.y).abs() > 16.0
                || (m.position.z - at.z).abs() > 16.0
            {
                continue;
            }
            if only_if_seen && !self.can_see(m.center(), eye) {
                continue;
            }
            let target_uuid = if universal {
                self.nearest_targetable_player_for_piglin(m)
                    .unwrap_or(player_uuid)
            } else {
                player_uuid
            };
            let id = self.common.mobs[i].id;
            let change = if mod_api::__private::listened(mod_api::HookId::LivingChangeTarget) {
                let original = self
                    .server
                    .lan_acting_entity
                    .or_else(|| self.player_identity().map(|(_, id)| id))
                    .and_then(|id| self.entity(id))
                    .and_then(|entity| self.living(entity));
                let Some(original) = original else { continue };
                let Some(epoch) = self.common.native_lifecycle.epoch() else {
                    continue;
                };
                let mob = mod_api::MobRef::new(mod_api::EntityRef::new(epoch, id));
                self.native_mob_target_event(mob, Some(original), true)
                    .and_then(|target| self.native_mob_bind_target(id, target))
                    .map_or(
                        sunrise_sim::mobs::TargetChange::Keep,
                        sunrise_sim::mobs::TargetChange::Set,
                    )
            } else {
                sunrise_sim::mobs::TargetChange::Set(Some(sunrise_sim::mobs::Who::Player))
            };
            // The piglin again by its id: one a listener removed is skipped.
            let Some(i) = self
                .common
                .mobs
                .iter()
                .position(|m| m.id == id && !m.removed)
            else {
                continue;
            };
            let m = &mut self.common.mobs[i];
            let previous = m.ai.target;
            sunrise_sim::mobs::piglin::set_guarded_anger_target(m, target_uuid);
            m.ai.target = match change {
                sunrise_sim::mobs::TargetChange::Keep => previous,
                sunrise_sim::mobs::TargetChange::Set(target) => target,
            };
        }
    }
    /// A clear line of sight between two points (blocks with collision stop it).
    fn can_see(&self, from: [f64; 3], to: [f64; 3]) -> bool {
        let steps = 32;
        (1..steps).all(|i| {
            let f = f64::from(i) / f64::from(steps);
            let p = [
                from[0] + (to[0] - from[0]) * f,
                from[1] + (to[1] - from[1]) * f,
                from[2] + (to[2] - from[2]) * f,
            ];
            !self
                .common
                .res
                .facts()
                .get(self.common.world.block(
                    p[0].floor() as i32,
                    p[1].floor() as i32,
                    p[2].floor() as i32,
                ))
                .full_collision
        })
    }
    /// Whether breaking or opening this block angers piglins.
    pub(in crate::game) fn guarded_by_piglins(&self, name: &str) -> bool {
        self.common
            .res
            .items()
            .block_tags
            .get("minecraft:guarded_by_piglins")
            .is_some_and(|t| t.contains(name))
    }
}

/// The decision of `Piglin.canReplaceCurrentItem` once both stacks are known.
fn piglin_replace(
    new_wanted: bool,
    current_wanted: bool,
    new: &equip::Facts,
    worn: &equip::Facts,
    slot: Slot,
    preferred: Option<(bool, bool)>,
) -> bool {
    if worn.prevents_armor_change {
        return false;
    }
    if new_wanted != current_wanted {
        return new_wanted;
    }
    equip::can_replace_current_item(new, Some(worn), slot, preferred)
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::piglins_common::PiglinsCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    use sunrise_gameplay_common::game::server_side::piglins::tests::armor;

    use super::*;

    #[test]
    fn better_helmet_replaces_worse_one() {
        // Diamond (3) over iron (2), neither loved.
        let (diamond, iron) = (armor(3.0, false), armor(2.0, false));
        assert!(piglin_replace(
            false,
            false,
            &diamond,
            &iron,
            Slot::Head,
            None
        ));
        assert!(!piglin_replace(
            false,
            false,
            &iron,
            &diamond,
            Slot::Head,
            None
        ));
    }

    #[test]
    fn binding_curse_keeps_the_worn_piece() {
        let (diamond, cursed) = (armor(3.0, false), armor(1.0, true));
        assert!(!piglin_replace(
            false,
            false,
            &diamond,
            &cursed,
            Slot::Head,
            None
        ));
        assert!(!piglin_replace(
            true,
            false,
            &diamond,
            &cursed,
            Slot::Head,
            None
        ));
    }

    #[test]
    fn wanted_beats_comparison() {
        let (gold, diamond) = (armor(2.0, false), armor(3.0, false));
        assert!(piglin_replace(
            true,
            false,
            &gold,
            &diamond,
            Slot::Head,
            None
        ));
        assert!(!piglin_replace(
            false,
            true,
            &diamond,
            &gold,
            Slot::Head,
            None
        ));
    }

    #[test]
    fn preferred_weapon_comparison_by_damage() {
        let mut strong = armor(0.0, false);
        strong.attack_damage = 7.0;
        let mut weak = armor(0.0, false);
        weak.attack_damage = 4.0;
        assert!(piglin_replace(
            false,
            false,
            &strong,
            &weak,
            Slot::MainHand,
            None
        ));
        assert!(!piglin_replace(
            false,
            false,
            &weak,
            &strong,
            Slot::MainHand,
            None
        ));
    }
}
