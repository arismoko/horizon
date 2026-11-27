//! The player's hands and what it wears: the stack in the acting hand (`getItemInHand`), the hand of
//! the item in use and running code with the other hand acting, `Inventory.add`, and the worn glider
//! and freeze-immune gear.
use super::*;

/// `Inventory.SLOT_OFFHAND`: the `using` slot of an off-hand item.
pub const OFFHAND_SLOT: usize = 40;

/// `AttackRange.effectiveMinRange`, `effectiveMaxRange` (for a player) and `hitboxMargin`.
pub fn attack_reach(range: &serde_json::Value, creative: bool) -> (f32, f32, f32) {
    let value = |key: &str, default: f32| range[key].as_f64().map_or(default, |v| v as f32);
    let (min, max) = if creative {
        (
            value("min_creative_reach", 0.0),
            value("max_creative_reach", 5.0),
        )
    } else {
        (value("min_reach", 0.0), value("max_reach", 3.0))
    };
    (min, max, value("hitbox_margin", 0.3))
}

/// `AttackRange.isInRange`: float bounds, then inclusive double comparison.
pub fn attack_range_contains(range: &serde_json::Value, distance: f64, creative: bool) -> bool {
    let (min, max, margin) = attack_reach(range, creative);
    in_attack_range(distance, (min, max, margin), 0.0)
}

/// `AttackRange.isInRange(attacker, box, buffer)` with inclusive eye-to-box distance.
pub fn in_attack_range(distance: f64, (min, max, margin): (f32, f32, f32), buffer: f64) -> bool {
    mod_api::ItemAttackRange {
        min,
        max,
        hitbox_margin: margin,
    }
    .contains(distance, buffer)
}

impl crate::game::CommonGame<'_> {
    /// The stack in an inventory slot, the off hand at `OFFHAND_SLOT`.
    pub fn stack_in(&self, slot: usize) -> Option<&ItemStack> {
        if slot == OFFHAND_SLOT {
            self.offhand.as_ref()
        } else {
            self.inventory.get(slot)?.as_ref()
        }
    }

    pub fn stack_in_mut(&mut self, slot: usize) -> &mut Option<ItemStack> {
        if slot == OFFHAND_SLOT {
            &mut self.offhand
        } else {
            &mut self.inventory[slot]
        }
    }

    /// The stack in the hand that acts (`getItemInHand(hand)`).
    pub fn held(&self) -> Option<&ItemStack> {
        self.stack_in(self.held_slot())
    }

    pub fn held_mut(&mut self) -> &mut Option<ItemStack> {
        self.stack_in_mut(self.held_slot())
    }

    /// The slot of the hand that acts: the selected one, or `OFFHAND_SLOT`.
    /// `LivingEntity.canGlide`: some worn piece, or a stack in a hand, glides
    /// there (`canGlideUsing` walks every `EquipmentSlot`: `gliding_slots`).
    pub fn glider_worn(&self) -> bool {
        super::glide_wear::gliding_slots(
            self.res.items(),
            &self.armor,
            self.inventory[self.selected].as_ref(),
            self.offhand.as_ref(),
        )
        .next()
        .is_some()
    }

    /// `LivingEntity.canFreeze`'s armor half: any worn piece in
    /// `freeze_immune_wearables`.
    pub fn wears_freeze_immune(&self) -> bool {
        let immune = self
            .res
            .items()
            .item_tags
            .get("minecraft:freeze_immune_wearables");
        self.armor
            .iter()
            .flatten()
            .any(|s| immune.is_some_and(|t| t.contains(&*s.item)))
    }

    pub fn held_slot(&self) -> usize {
        match self.hand {
            Hand::Main => self.selected,
            Hand::Off => OFFHAND_SLOT,
        }
    }

    /// `LivingEntity.getUsedItemHand`: the hand of the item in use.
    pub fn used_hand(&self) -> Option<Hand> {
        item_use_common::hand_of(self.using)
    }

    fn insert_stack_into_player_slots(&mut self, stack: &mut ItemStack, max: u32) -> bool {
        inventory_add(
            &mut self.inventory,
            &mut self.offhand,
            self.selected,
            max,
            stack,
        )
    }

    /// `getAttributeValue` of an interaction range of the acting player.
    pub fn interaction_range(&mut self, attribute: &str, survival: f64, creative: f64) -> f64 {
        let otherwise = if self.creative { creative } else { survival };
        self.player
            .as_mut()
            .and_then(|p| p.attribute(attribute).ok())
            .unwrap_or(otherwise)
    }

    /// `Player.entityInteractionRange`/`blockInteractionRange` for the tick's picks.
    pub fn update_interaction_ranges(&mut self) {
        self.entity_reach = self.interaction_range("minecraft:entity_interaction_range", 3.0, 5.0);
        self.block_reach = self.interaction_range("minecraft:block_interaction_range", 4.5, 5.0);
    }

    /// A stack's `minecraft:attack_range` component, with spear default fallback unless removed.
    pub fn attack_range_component(&self, stack: &ItemStack) -> Option<serde_json::Value> {
        let view_stack = sunrise_sim::item_models::stack_view(stack);
        let view = self.pack.item_models.components_view(&view_stack);
        if let Some(val) = view.get("minecraft:attack_range") {
            return Some(val.clone());
        }
        if stack
            .data()
            .is_some_and(|d| d.other("!minecraft:attack_range").is_some())
        {
            return None;
        }
        sunrise_sim::spear_sim::Kinetic::of(&stack.item).map(|_| {
            serde_json::json!({
                "min_reach": sunrise_sim::spear_sim::REACH.min,
                "max_reach": sunrise_sim::spear_sim::REACH.max,
                "min_creative_reach": sunrise_sim::spear_sim::REACH.min_creative,
                "max_creative_reach": sunrise_sim::spear_sim::REACH.max_creative,
                "hitbox_margin": sunrise_sim::spear_sim::REACH.hitbox_margin,
            })
        })
    }

    /// `LocalPlayer.raycastHitResult`'s active item `attack_range` component.
    pub fn active_attack_range(&self) -> Option<serde_json::Value> {
        let active = self
            .using
            .and_then(|(slot, _)| self.inventory.get(slot).and_then(Option::as_ref))
            .or_else(|| self.held())?;
        self.attack_range_component(active)
    }

    /// `Attributes.ATTACK_SPEED` sanitizer from the attribute catalog.
    pub fn sanitize_attack_speed(&self, value: f64) -> f64 {
        self.motion
            .attributes
            .definition("minecraft:attack_speed")
            .map_or(value, |definition| definition.sanitize(value))
    }

    /// `Entity.getKnownSpeed` of the mob at `index`, per second: its root vehicle's.
    pub fn mob_speed(&self, index: usize) -> Vec3 {
        let mut mob = &self.mobs[index];
        while let Some(vehicle) = mob
            .extra
            .mount
            .and_then(|v| self.mobs.iter().find(|m| m.id == v))
        {
            mob = vehicle;
        }
        mob.position.subtract(mob.previous).scale(20.0)
    }

    /// The armour (head first) and off hand in `EquipmentSlot` order.
    #[allow(dead_code)]
    pub(in crate::game) fn player_equipment(&self) -> [Option<ItemStack>; 5] {
        let [head, chest, legs, feet] = self.armor.clone();
        [head, chest, legs, feet, self.offhand.clone()]
    }

    /// `ThrowableProjectile.tick`'s `applyGravity` and `applyInertia`.
    pub fn thrown_potion_motion(&self, t: &mut super::potions_common::ThrownPotion) {
        let gravity = if &*t.stack.item == "minecraft:experience_bottle" {
            0.07
        } else {
            0.05
        };
        let at = t.position;
        let water = self
            .res
            .facts()
            .get(self.world.block(
                at.x.floor() as i32,
                at.y.floor() as i32,
                at.z.floor() as i32,
            ))
            .fluid
            == sunrise_items::blocks::Fluid::Water;
        let inertia = f64::from(if water { 0.8_f32 } else { 0.99_f32 });
        t.velocity = Vec3::new(
            t.velocity.x * inertia,
            (t.velocity.y - gravity) * inertia,
            t.velocity.z * inertia,
        );
    }

    /// Whether the stack has `DataComponents.DEATH_PROTECTION`, whatever item it is.
    pub fn has_death_protection(&self, s: &ItemStack) -> bool {
        super::totem_common::death_protection_of(&self.pack.item_models, &s.item, Some(s)).is_some()
    }

    /// `Attributes.MINING_EFFICIENCY`, which `Player.getDestroySpeed` adds to tool speed > 1.
    pub(in crate::game) fn mining_efficiency(&mut self) -> f32 {
        self.player
            .as_mut()
            .and_then(|player| player.attribute("minecraft:mining_efficiency").ok())
            .unwrap_or(0.0) as f32
    }

    /// `Player.getDestroySpeed`'s final `Attributes.BLOCK_BREAK_SPEED` factor.
    pub(in crate::game) fn block_break_speed(&mut self) -> f32 {
        self.player
            .as_mut()
            .and_then(|p| p.attribute("minecraft:block_break_speed").ok())
            .map_or(1.0, |v| v as f32)
    }

    /// Stops player sprinting after extra knockback.
    pub fn stop_sprint_after_extra_knockback(&mut self) {
        if let Some(p) = self.player.as_mut() {
            let _ = p.set_sprinting(false);
        }
    }

    /// Horizontal attacker slowdown and sprint stop after `causeExtraKnockback`.
    pub fn attacker_slowdown(&mut self) {
        if let Some(player) = self.player.as_mut() {
            player.velocity.x *= sunrise_sim::attack_strength::ATTACKER_SLOWDOWN;
            player.velocity.z *= sunrise_sim::attack_strength::ATTACKER_SLOWDOWN;
        }
        self.stop_sprint_after_extra_knockback();
    }

    /// The weapon's source override, item override, then the actor's ordinary melee source.
    pub fn stack_melee_damage_type(
        &self,
        stack: Option<&ItemStack>,
        (fall, flying): (f64, bool),
        player: bool,
    ) -> String {
        if let Some(value) = stack.and_then(|s| {
            self.pack
                .item_models
                .components_view(&sunrise_sim::item_models::stack_view(s))
                .get("minecraft:damage_type")
                .cloned()
        }) && let Some(id) = value.as_str()
        {
            return sunrise_core::identifier::Identifier::parse(id)
                .map_or_else(|_| id.to_owned(), |id| id.to_string());
        }
        if stack.is_some_and(|s| &*s.item == "minecraft:mace")
            && super::mace_common::can_smash(fall, flying)
        {
            "minecraft:mace_smash".into()
        } else if player {
            "minecraft:player_attack".into()
        } else {
            "minecraft:mob_attack".into()
        }
    }

    /// `Player.isWithinAttackRange(mainHandItem, box, 3.0)` for the player in place.
    pub fn within_attack_range(&mut self, target: &sunrise_core::geometry::Aabb) -> bool {
        let Some(eye) = self.eye_position() else {
            return false;
        };
        let stack = self.held().cloned();
        let default = self.interaction_range("minecraft:entity_interaction_range", 3.0, 5.0);
        let reach = self.stack_attack_range(stack.as_ref(), true, self.creative, default);
        let distance = target
            .distance_squared_to_point(Vec3::new(eye[0], eye[1], eye[2]))
            .sqrt();
        reach.contains(distance, 3.0)
    }

    /// The same effective component view for a detached stack and normal held-stack attacks.
    pub(in crate::game) fn stack_attack_range(
        &self,
        stack: Option<&ItemStack>,
        player: bool,
        creative: bool,
        default: f64,
    ) -> mod_api::ItemAttackRange {
        let range = stack.and_then(|stack| {
            self.pack
                .item_models
                .components_view(&sunrise_sim::item_models::stack_view(stack))
                .get("minecraft:attack_range")
                .cloned()
        });
        let (mut min, mut max, hitbox_margin) = range
            .as_ref()
            .map_or((0.0, default as f32, 0.0), |r| attack_reach(r, creative));
        if !player && let Some(range) = &range {
            let factor = range["mob_factor"].as_f64().unwrap_or(1.0) as f32;
            min *= factor;
            max *= factor;
        }
        mod_api::ItemAttackRange {
            min,
            max,
            hitbox_margin,
        }
    }

    /// `Player.isWithinEntityInteractionRange(box, buffer)` for the player in place.
    pub fn within_entity_interaction_range(
        &mut self,
        target: &sunrise_core::geometry::Aabb,
        buffer: f64,
    ) -> bool {
        let Some(eye) = self.eye_position() else {
            return false;
        };
        let range = self.interaction_range("minecraft:entity_interaction_range", 3.0, 5.0) + buffer;
        target.distance_squared_to_point(Vec3::new(eye[0], eye[1], eye[2])) < range * range
    }

    /// `EntityFluidInteraction`: compare the eyes with the camera fluid surface.
    pub(in crate::game) fn mining_eyes_in_water(&self) -> bool {
        use sunrise_items::blocks::Fluid;
        let Some(eye) = self.eye_position() else {
            return false;
        };
        let [x, y, z] = eye.map(|v| v.floor() as i32);
        let facts = self.res.facts();
        let inside = |y| {
            let fluid = facts.get(self.world.block(x, y, z));
            if fluid.fluid != Fluid::Water {
                return false;
            }
            let above = facts.get(self.world.block(x, y + 1, z));
            let source = fluid.fluid_level == 8 && !fluid.fluid_falling;
            let height = if above.fluid == fluid.fluid || (source && above.sturdy_full & 1 != 0) {
                1.0
            } else {
                f64::from(f32::from(fluid.fluid_level) / 9.0)
            };
            eye[1] <= f64::from(y) + height
        };
        inside(y) || (eye[1] == f64::from(y) && inside(y - 1))
    }

    /// The main-hand stack a break begins with (`destroyingItem`).
    pub fn main_hand_key(&self) -> Option<ItemStack> {
        self.held().cloned()
    }

    pub fn charge_ticks(stack: &ItemStack) -> u32 {
        // crossbow_charge_time: 1.25 s, minus 0.25 s per Quick Charge level.
        let seconds = 1.25 - 0.25 * stack.enchantment_level("minecraft:quick_charge") as f32;
        (seconds.max(0.0) * 20.0).floor() as u32
    }

    /// The hook's position this frame.
    pub fn hook_position(&self, alpha: f64) -> Option<glam::DVec3> {
        let h = self.hook.as_ref()?;
        let lerp = |a: f64, b: f64| a + (b - a) * alpha;
        Some(glam::DVec3::new(
            lerp(h.previous.x, h.position.x),
            lerp(h.previous.y, h.position.y),
            lerp(h.previous.z, h.position.z),
        ))
    }

    /// The hook for `sunrise-ctl status`.
    pub fn hook_status(&self) -> Option<serde_json::Value> {
        let h = self.hook.as_ref()?;
        Some(serde_json::json!({
            "state": format!("{:?}", h.state),
            "nibble": h.nibble,
            "lured_in": h.time_until_lured,
            "hooked_in": h.time_until_hooked,
            "open_water": h.open_water,
            "loot_table": h.medium.as_ref().and_then(|m| m.loot_table.clone()),
        }))
    }

    pub fn water_height(&self, x: i32, y: i32, z: i32) -> Option<(f32, bool)> {
        let f = self.res.facts().get(self.world.block(x, y, z));
        if f.fluid != sunrise_items::blocks::Fluid::Water {
            return None;
        }
        let above = self.res.facts().get(self.world.block(x, y + 1, z));
        let h = if above.fluid == sunrise_items::blocks::Fluid::Water {
            1.0
        } else {
            f32::from(f.fluid_level) / 9.0
        };
        Some((h, f.fluid_level == 8 && !f.fluid_falling))
    }

    /// `FishingHook.checkCollision` clips blocks on both sides of the connection.
    pub fn hook_block_hit(&self, from: Vec3, to: Vec3) -> Option<sunrise_core::geometry::BlockHit> {
        let mut scene = sunrise_level::collisions::SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        sunrise_core::geometry::traverse_blocks(
            from,
            to,
            1024,
            |p: sunrise_core::position::BlockPos| {
                let state = self.world.block(p.x, p.y, p.z);
                if self.res.facts().is_air(state) {
                    return None;
                }
                self.collision
                    .collision_shape(&mut scene, state, [p.x, p.y, p.z])
                    .ok()?
                    .clip(from, to, p)
            },
        )
        .ok()
        .flatten()
    }
}

pub trait HeldItemsGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Run `f` with `hand` acting (`held`, `consume_held`, `swing_arm`).
    fn with_hand<T>(&mut self, hand: Hand, f: impl FnOnce(&mut Self) -> T) -> T {
        let prior = std::mem::replace(&mut self.hand, hand);
        let out = f(self);
        self.hand = prior;
        out
    }

    /// Inventory.add: returns the stack that did not fit.
    fn add_stack_to_inventory(&mut self, mut stack: ItemStack) -> Option<ItemStack> {
        self.insert_stack_into_inventory(&mut stack)
            .then_some(stack)
    }

    /// Consumes the fitted count, leaving the offered item and components in `stack`.
    /// Returns true if some of the count remains.
    fn insert_stack_into_inventory(&mut self, stack: &mut ItemStack) -> bool {
        let max = self
            .res
            .items()
            .get(&stack.item)
            .map_or(64, |i| i.max_stack);
        self.insert_stack_up_to(stack, max)
    }

    /// `insert_stack_into_inventory` with `max` the stack's own maximum.
    fn insert_stack_up_to(&mut self, stack: &mut ItemStack, max: u32) -> bool {
        let before = stack.count;
        let left = self.insert_stack_into_player_slots(stack, max);
        if stack.count < before {
            self.side_on_inventory_item_added(stack);
        }
        left
    }

    /// Shared equipment base-attribute modifier pass used by both server and client roles.
    fn apply_equipment_modifiers_with(&mut self, applied: &mut Vec<(String, String)>) -> bool {
        if self.player.is_none() {
            return false;
        }
        let items = self.items();
        let slots = ["head", "chest", "legs", "feet"];
        let mut to_add: Vec<(String, String, f64, String)> = Vec::new();
        let equipped = self
            .armor
            .iter()
            .zip(slots)
            .map(|(s, n)| (s.as_ref(), n))
            .chain([
                (self.inventory[self.selected].as_ref(), "mainhand"),
                (self.offhand.as_ref(), "offhand"),
            ]);
        for (stack, slot) in equipped {
            let Some(stack) = stack else { continue };
            for row in sunrise_sim::item_attributes::attribute_modifiers(stack, items).iter() {
                if sunrise_sim::living_attributes::modifier_applies(&row.slot, slot) {
                    to_add.push((
                        row.attribute.clone(),
                        row.id.clone(),
                        row.amount,
                        row.operation.clone(),
                    ));
                }
            }
        }
        let Some(player) = self.player.as_mut() else {
            return false;
        };
        for (attribute, id) in std::mem::take(applied) {
            let _ = player.set_modifier_unclamped(&attribute, &id, 0.0, "add_value", false);
        }
        for (attribute, id, amount, op) in to_add {
            if player
                .set_modifier_unclamped(&attribute, &id, amount, &op, true)
                .is_ok()
            {
                applied.push((attribute, id));
            }
        }
        true
    }

    /// `Player.resetAttackStrengthTicker`.
    fn reset_attack_strength(&mut self) {
        self.side_reset_attack_strength();
    }

    /// `Attributes.ATTACK_SPEED` with main-hand and haste/fatigue modifiers.
    fn main_hand_attack_speed(&self) -> f64 {
        self.side_main_hand_attack_speed()
    }

    /// `ServerPlayer.getKnownMovement` of the acting or window player.
    fn known_player_movement(&self) -> Vec3 {
        self.side_known_player_movement()
    }

    /// Equipment attribute modifiers (`LivingEntity.collectEquipmentChanges`).
    fn apply_equipment(&mut self) {
        self.side_apply_equipment();
    }

    // ---- shared potions, totem, combat, mining, consume, item use, crossbow, buckets, trident, fishing, recipe book ----

    /// Effects' armor protection and status flags on the current player.
    fn apply_effect_status(&mut self) {
        self.refresh_armor_protection();
        let mut status = self.potions.effects.status();
        if let Some(p) = self.player.as_mut() {
            status.protection = std::mem::take(&mut p.status.protection);
            p.status = status;
        }
    }

    /// `EnchantmentHelper.getDamageProtection` after equipment changes.
    fn refresh_armor_protection(&mut self) {
        let armor: Vec<Option<ItemStack>> = self.armor.to_vec();
        if !self
            .potions
            .protection_for
            .as_ref()
            .is_some_and(|(generation, cached)| {
                *generation == self.damage_tag_generation && cached == &armor
            })
        {
            let mut table = Vec::new();
            let e = &self.stations().enchantments;
            let mut random = sunrise_core::random::LegacyRandom::new(0);
            for (_, def) in self.health_resources.damage.entries() {
                let ctx = sunrise_items::enchant::EffectContext {
                    items: Some(self.items()),
                    damage_type: Some(&def.name),
                    this: Some("minecraft:player"),
                    ..Default::default()
                };
                let total: f32 = armor
                    .iter()
                    .flatten()
                    .map(|s| e.preview_value(s, "minecraft:damage_protection", &ctx, &mut random))
                    .sum();
                if total > 0.0 {
                    table.push((def.name.clone(), total));
                }
            }
            self.potions.protection_for = Some((self.damage_tag_generation, armor));
            if let Some(p) = self.player.as_mut() {
                p.status.protection = table;
            }
        }
    }

    /// `Attributes.SUBMERGED_MINING_SPEED` (0.2, raised by aqua affinity).
    fn submerged_mining_speed(&self) -> f32 {
        let mut total = 0.0;
        for stack in self.armor.iter().flatten() {
            for (attr, _, amount, op) in self.stations().enchantments.attributes(stack) {
                if attr == "minecraft:submerged_mining_speed" && op == "add_multiplied_total" {
                    total += amount;
                }
            }
        }
        (0.2 * (1.0 + total)) as f32
    }

    /// `Minecraft.startAttack` on an entity: main-hand `attack_range` gate.
    fn held_range_allows(&self, distance: f64) -> bool {
        let range = self
            .held()
            .and_then(|stack| self.attack_range_component(stack));
        range.is_none_or(|r| attack_range_contains(&r, distance, self.own_player_is_creative()))
    }

    /// The stack's `blocks_attacks`, gated by `shield_block` item abilities.
    fn blocks_attacks<'__loan>(
        &'__loan self,
        stack: &ItemStack,
    ) -> Option<std::borrow::Cow<'__loan, sunrise_items::items::BlocksAttacks>>
    where
        '__sg: '__loan,
    {
        let component =
            sunrise_items::items::BlocksAttacks::of_stack(stack, self.items().get(&stack.item));
        let ability = super::item_abilities::ability::SHIELD_BLOCK;
        let blocks = self.stack_can_perform(stack, ability, component.is_some());
        match component {
            Some(component) if blocks => Some(component),
            None if blocks => Some(std::borrow::Cow::Borrowed(
                sunrise_items::items::BlocksAttacks::bare(),
            )),
            _ => None,
        }
    }

    /// Whether `kind` belongs to `#minecraft:redirectable_projectile`.
    fn is_redirectable_projectile_type(&self, kind: &str) -> bool {
        if let Some(answer) = self.side_redirectable_projectile_tag(kind) {
            return answer;
        }
        let tags = &self.items().entity_type_tags;
        if let Some(members) = tags.members().get("minecraft:redirectable_projectile") {
            return members.contains(kind);
        }
        matches!(
            kind,
            "minecraft:fireball" | "minecraft:wind_charge" | "minecraft:breeze_wind_charge"
        )
    }

    /// `Projectile.isPickable` (with `ShulkerBullet` and `AbstractArrow` overrides).
    fn projectile_is_pickable(&self, p: &sunrise_sim::projectiles::Projectile) -> bool {
        if matches!(
            p.kind,
            sunrise_sim::projectiles::ProjectileKind::ShulkerBullet { .. }
        ) {
            return true;
        }
        if p.is_arrow() && p.stuck {
            return false;
        }
        self.is_redirectable_projectile_type(p.entity_type())
    }

    /// `Projectile.getPickRadius`: `this.isPickable() ? 1.0F : 0.0F`.
    fn projectile_pick_radius(&self, p: &sunrise_sim::projectiles::Projectile) -> f64 {
        if self.projectile_is_pickable(p) {
            super::deflect_common::PICK_RADIUS
        } else {
            0.0
        }
    }

    /// `AbstractArrow.isPickable` for `ThrownTrident`.
    fn trident_is_pickable(&self, t: &sunrise_sim::thrown_trident::ThrownTrident) -> bool {
        !t.in_ground && self.is_redirectable_projectile_type("minecraft:trident")
    }

    /// `Projectile.isPickable` for `FireworkRocketEntity`.
    fn rocket_is_pickable(&self) -> bool {
        self.is_redirectable_projectile_type("minecraft:firework_rocket")
    }

    /// `IBlockExtension.canHarvestBlock` followed by `PlayerEvent.HarvestCheck`.
    fn mod_harvest_check(
        &mut self,
        pos: [i32; 3],
        state: sunrise_world::registry::BlockStateId,
        harvest: &mut bool,
    ) {
        let player = self.own_entity_id() as u32;
        let _dispatching = HarvestDispatch::enter();
        self.native_can_harvest_block(player, pos, state, harvest);
        if mod_api::__private::listened(mod_api::HookId::PlayerHarvestCheck) {
            self.native_player_harvest_check(player, pos, state.0, harvest);
        }
    }

    /// Whether the acting player's selected hotbar slot harvests `state` at `pos`.
    fn acting_can_harvest(
        &mut self,
        pos: [i32; 3],
        state: sunrise_world::registry::BlockStateId,
    ) -> bool {
        let facts = self.res.facts().get(state);
        let tool = self.stack_in(self.selected);
        let mut harvest = !facts.requires_tool || self.items().correct_for_drops(tool, &facts.name);
        self.mod_harvest_check(pos, state, &mut harvest);
        harvest
    }

    /// `BlockBehaviour.getDestroyProgress` with `Player.getDestroySpeed`.
    fn destroy_progress(&mut self, pos: [i32; 3]) -> f32 {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        self.destroy_progress_for(pos, state)
    }

    /// `BlockBehaviour.getDestroyProgress` for a supplied state at `pos`: a block Part's
    /// `destroy_progress` answer, or the game's own dig math, including the
    /// `player_break_speed` hook. The caller checks that `state` is a known state when it
    /// comes from a mod.
    fn destroy_progress_for(
        &mut self,
        pos: [i32; 3],
        state: sunrise_world::registry::BlockStateId,
    ) -> f32 {
        // The block's own `getDestroyProgress`, which a Part's answer overrides whole. Inside
        // that answer a query gives the game's math, as the default method below would.
        if !DestroyProgressDispatch::running()
            && crate::game::api_values::block_state::block_listens(
                mod_api::HookId::DestroyProgress,
                state,
            )
            && let Some(player) = self.side_destroy_progress_player_ref()
        {
            let mut answer = None;
            let _dispatching = DestroyProgressDispatch::enter();
            self.native_destroy_progress(player, pos, state, &mut answer);
            if let Some(progress) = answer {
                return progress;
            }
        }
        let facts = self.res.facts().get(state);
        if facts.hardness == -1.0 {
            return 0.0;
        }
        let hardness = facts.hardness;
        let harvest = self.acting_can_harvest(pos, state);
        let block_break_speed = self.block_break_speed();
        let facts = self.res.facts().get(state);
        let tool_speed = self.items().mining_speed(self.held(), &facts.name);
        let player = sunrise_items::mining::DestroySpeedModifiers {
            mining_efficiency: if tool_speed > 1.0 {
                self.mining_efficiency()
            } else {
                0.0
            },
            effects: self.potions.effects.mining_speed_factor(),
            block_break_speed,
            submerged_mining_speed: self
                .mining_eyes_in_water()
                .then(|| self.submerged_mining_speed()),
            on_ground: self.player.as_ref().is_none_or(|p| p.ground.on_ground),
        };
        let mut speed = sunrise_items::mining::destroy_speed(tool_speed, &player);
        let mut canceled = false;
        if mod_api::__private::listened(mod_api::HookId::PlayerBreakSpeed) {
            let _dispatching = BreakSpeedDispatch::enter();
            self.native_player_break_speed(
                self.own_entity_id() as u32,
                pos,
                state.0,
                speed,
                &mut speed,
                &mut canceled,
            );
        }
        if canceled {
            return 0.0;
        }
        sunrise_items::mining::destroy_progress(hardness, harvest, speed)
    }

    /// `stack.get(DataComponents.CONSUMABLE)`: item prototype, data patch or wire patch.
    fn consumable(
        &self,
        stack: &ItemStack,
    ) -> Option<std::sync::Arc<super::item_use_common::ItemConsumable>> {
        stack.consumable(self.items().get(&stack.item)).cloned()
    }

    /// `Consumable.emitParticlesAndSounds(user.getRandom(), user, stack, count)`.
    fn emit_consume(
        &mut self,
        consumable: &super::item_use_common::ItemConsumable,
        stack: &ItemStack,
        count: i32,
    ) {
        self.side_emit_consume(consumable, stack, count);
    }

    /// Resolve an item's animation once for the caller's rendering decision.
    fn item_use_animation(&self, stack: &ItemStack) -> sunrise_core::use_animation::UseAnimation {
        self.item_use_state(stack).animation()
    }

    /// `ItemStack::use_animation`: the renderer's own read, none for an unknown item.
    fn item_stack_use_animation(
        &self,
        stack: &ItemStack,
    ) -> Option<sunrise_core::use_animation::UseAnimation> {
        self.items().get(&stack.item)?;
        Some(self.item_use_animation(stack))
    }

    /// Select the animation and its already decoded spear timing together.
    fn item_use_state(&self, stack: &ItemStack) -> super::item_use_common::ItemUseState {
        use super::item_use_common::{ASKING_USE_ANIMATION, AskingUseAnimation, ItemUseState};
        use sunrise_core::use_animation::UseAnimation;
        let info = self.items().get(&stack.item);
        let kinetic =
            sunrise_sim::spear_sim::UseTiming::of_stack(stack, info.and_then(|i| i.kinetic_timing));
        let fixed = match &*stack.item {
            "minecraft:bow" => Some(UseAnimation::Bow),
            "minecraft:crossbow" => Some(UseAnimation::Crossbow),
            "minecraft:trident" => Some(UseAnimation::Trident),
            "minecraft:brush" => Some(UseAnimation::Brush),
            "minecraft:spyglass" => Some(UseAnimation::Spyglass),
            "minecraft:goat_horn" => Some(UseAnimation::TootHorn),
            _ if info.is_some_and(|i| i.bundle) => Some(UseAnimation::Bundle),
            _ => None,
        };
        let data = stack.data();
        let mut animation = fixed.unwrap_or_else(|| {
            if let Some(consumable) = self.consumable(stack) {
                consumable.animation
            } else if self.stack_can_perform(
                stack,
                super::item_abilities::ability::SHIELD_BLOCK,
                data.is_some_and(|d| d.other("minecraft:blocks_attacks").is_some())
                    || (data.is_none_or(|d| d.other("!minecraft:blocks_attacks").is_none())
                        && info.is_some_and(|i| i.blocks_attacks.is_some())),
            ) {
                UseAnimation::Block
            } else if kinetic.is_some() {
                UseAnimation::Spear
            } else {
                UseAnimation::None
            }
        });
        if !ASKING_USE_ANIMATION.with_borrow(|asking| asking.contains(&stack.item)) {
            ASKING_USE_ANIMATION.with_borrow_mut(|asking| asking.push(stack.item.clone()));
            let _asking = AskingUseAnimation;
            self.native_use_animation(stack, &mut animation);
        }
        if animation == UseAnimation::Spear {
            return kinetic.map_or(ItemUseState::Other(UseAnimation::None), ItemUseState::Spear);
        }
        ItemUseState::Other(animation)
    }

    /// `LivingEntity.startUsingItem` for the acting player's hand.
    fn start_using_item(&mut self) -> bool {
        let Some(stack) = self.held().cloned().filter(|_| self.using.is_none()) else {
            return false;
        };
        let own = self.use_duration(&stack);
        let (mut duration, mut canceled) = (own, false);
        let hand = self.hand;
        self.native_living_entity_use_item_start(
            &stack,
            Some(self.own_entity_id()),
            hand,
            &mut duration,
            &mut canceled,
        );
        let Some(ticks_set) = super::item_use_common::start_of(own, duration, canceled) else {
            return false;
        };
        self.using = Some((self.held_slot(), 0));
        self.use_duration = duration;
        self.use_start = super::item_use_common::UseStart {
            ticks_set,
            item: Some(stack.clone()),
        };
        self.side_on_start_using_item(&stack);
        true
    }

    /// `ItemStack.getUseDuration`.
    fn use_duration(&self, stack: &ItemStack) -> i32 {
        let info = self.items().get(&stack.item);
        let (patched_ticks, patched_blocks) = sunrise_sim::item_sync::patched_use(stack);
        let mut duration = match &*stack.item {
            "minecraft:bow" | "minecraft:crossbow" | "minecraft:trident" => 72_000,
            "minecraft:spyglass" => 1200,
            "minecraft:brush" => 200,
            _ if info.is_some_and(|i| i.bundle) => 200,
            "minecraft:ender_eye" => 0,
            "minecraft:goat_horn" => self.native_item_instrument_duration(stack),
            _ => {
                if let Some(ticks) = patched_ticks {
                    ticks
                } else if let Some(consumable) = self.consumable(stack) {
                    consumable.ticks()
                } else if patched_blocks
                    || self.stack_carries_blocks_attacks(stack, info)
                    || stack.carries_component(info, "minecraft:kinetic_weapon")
                {
                    72_000
                } else {
                    0
                }
            }
        };
        self.native_use_duration(stack, &mut duration);
        duration
    }

    /// Whether `stack` is a mod's item that only a mod uses over time.
    fn mod_use(&self, stack: &ItemStack) -> bool {
        let info = self.items().get(&stack.item);
        mod_api::__private::is_content_item(&stack.item)
            && info.is_some()
            && self.consumable(stack).is_none()
            && !self.stack_carries_blocks_attacks(stack, info)
            && !stack.carries_component(info, "minecraft:kinetic_weapon")
            && self.use_duration(stack) > 0
    }

    /// A right click with a `mod_use` item: `startUsingItem` (CONSUME).
    fn start_mod_use(&mut self, stack: &ItemStack) -> bool {
        if !self.mod_use(stack) {
            return false;
        }
        self.start_using_item();
        true
    }

    /// `LivingEntityUseItemEvent.Tick` for `stack` in use `ticks` so far.
    fn use_item_ticked(&mut self, stack: &ItemStack, ticks: u32) -> bool {
        let before = super::item_use_common::ticks_left(self.use_duration, ticks);
        let (mut remaining, mut canceled) = (before, false);
        let hand = self.used_hand().unwrap_or(self.hand);
        self.native_living_entity_use_item_tick(
            stack,
            Some(self.own_entity_id()),
            hand,
            &mut remaining,
            &mut canceled,
        );
        let (left, set) = super::item_use_common::tick_left(
            before,
            remaining,
            canceled,
            self.use_start.ticks_set,
        );
        self.use_start.ticks_set = set;
        self.use_duration = super::item_use_common::duration_leaving(ticks, left);
        left > 0
    }

    /// A mod's `Item.onUseTick` of `stack` used `ticks`, while ticks are left.
    fn item_on_use_tick(&mut self, stack: &ItemStack, ticks: u32) -> bool {
        let using = self.using;
        let remaining = super::item_use_common::ticks_left(self.use_duration, ticks);
        if remaining > 0 {
            self.native_on_use_tick(stack, Some(self.own_entity_id()), remaining);
        }
        self.using == using
    }

    /// How long `stack`, used `ticks`, counts as held.
    fn ticks_held(&self, stack: &ItemStack, ticks: u32) -> u32 {
        super::item_use_common::held(
            self.use_duration(stack),
            super::item_use_common::ticks_left(self.use_duration, ticks),
        )
    }

    /// `LivingEntityUseItemEvent.Stop` as `releaseUsingItem` lets go of `stack`.
    fn use_item_stopped(&mut self, stack: &ItemStack, ticks: u32) -> bool {
        let mut canceled = false;
        let remaining = super::item_use_common::ticks_left(self.use_duration, ticks);
        let hand = self.hand;
        self.native_living_entity_use_item_stop(
            stack,
            Some(self.own_entity_id()),
            hand,
            remaining,
            &mut canceled,
        );
        !canceled
    }

    /// A mod's `Item.releaseUsing` of `stack` used `ticks`.
    fn mod_release_using(&mut self, stack: &ItemStack, ticks: u32) {
        let mut used = false;
        let remaining = super::item_use_common::ticks_left(self.use_duration, ticks);
        self.native_release_using(stack, Some(self.own_entity_id()), remaining, &mut used);
        if used {
            self.side_apply_use_cooldown(stack);
        }
    }

    /// `Player.canEat(canAlwaysEat)` for `stack`.
    fn can_eat(&self, stack: Option<&ItemStack>, can_always_eat: bool) -> bool {
        let vanilla = can_always_eat
            || self.creative
            || self
                .player
                .as_ref()
                .is_some_and(|p| p.health.food.food < 20);
        if !mod_api::__private::enabled_listened(mod_api::HookId::CanEat) {
            return vanilla;
        }
        let Some(player) = self.side_acting_player_ref() else {
            return vanilla;
        };
        let mut answer = None;
        self.native_can_eat(player, stack, vanilla, &mut answer);
        answer.unwrap_or(vanilla)
    }

    /// `Player.getProjectile` for `weapon` (`projectile_slot`).
    fn projectile_slot(&self, weapon: &str) -> Option<usize> {
        let hands = [OFFHAND_SLOT, self.selected].map(|slot| (slot, self.stack_in(slot)));
        projectile_slot(weapon, hands, &self.inventory, self.items())
    }

    /// `Player.getProjectile` for `weapon`, as `LivingGetProjectileEvent` leaves it.
    fn get_projectile(&mut self, weapon: &ItemStack) -> Option<super::crossbow_common::Ammo> {
        let slot = self.projectile_slot(&weapon.item);
        let vanilla = match slot {
            Some(i) => self.stack_in(i).cloned(),
            None => self
                .creative
                .then(|| ItemStack::new(super::crossbow_common::ARROW, 1)),
        };
        if !mod_api::__private::listened(mod_api::HookId::LivingGetProjectile) {
            return vanilla.map(|stack| super::crossbow_common::Ammo { stack, slot });
        }
        let mut projectile = vanilla.clone();
        let player = self.side_projectile_player_uuid();
        let name = self.own_profile_name();
        self.native_living_get_projectile(
            player,
            &name,
            None,
            "minecraft:player",
            weapon,
            &mut projectile,
        );
        let slot = slot.filter(|_| projectile == vanilla);
        projectile.map(|stack| super::crossbow_common::Ammo { stack, slot })
    }

    /// `getPlayerPOVHitResult(level, player, SOURCE_ONLY)` for an empty bucket.
    fn bucket_hit(&self) -> Option<Hit> {
        let origin = self.aim().map_or([0.0; 3], |(origin, _)| origin);
        let distance = |p: [f64; 3]| {
            (p[0] - origin[0]).powi(2) + (p[1] - origin[1]).powi(2) + (p[2] - origin[2]).powi(2)
        };
        match (self.side_item_pick(), self.pick_fluid()) {
            (Some(block), Some(fluid)) => Some(if distance(block.point) <= distance(fluid.point) {
                block
            } else {
                fluid
            }),
            (block, fluid) => block.or(fluid),
        }
    }

    /// `BucketItem.use` target block position.
    fn bucket_target(
        &mut self,
        content: sunrise_world::level::BucketContent,
        hit: &Hit,
    ) -> [i32; 3] {
        let [x, y, z] = hit.pos;
        let clicked = self.world.block(x, y, z);
        if content == sunrise_world::level::BucketContent::Powder {
            if self.res.facts().get(clicked).replaceable {
                return hit.pos;
            }
        } else if let Some(fluid) = content.fluid() {
            let user = self.bucket_liquid_user();
            if self.with_level(|b, level, _| {
                b.bucket_can_fill(
                    level,
                    sunrise_core::position::BlockPos::new(x, y, z),
                    clicked,
                    fluid,
                    user,
                )
            }) {
                return hit.pos;
            }
        }
        hit.beside()
    }

    fn bucket_liquid_user(&self) -> sunrise_world::level::LiquidUser {
        sunrise_world::level::LiquidUser {
            entity_id: self.side_bucket_liquid_user_entity_id(),
            creative: self.creative,
        }
    }

    /// The trident stack's maximum damage component, else its item's default.
    fn trident_max_damage(&self, stack: &ItemStack) -> u32 {
        stack
            .data()
            .and_then(|data| match data.other("minecraft:max_damage") {
                Some(sunrise_core::nbt::Tag::Int(value)) => u32::try_from(*value).ok(),
                _ => None,
            })
            .unwrap_or_else(|| self.items().get(&stack.item).map_or(250, |i| i.max_damage))
    }

    /// `TridentItem.use`: start the throw unless one more use would break it.
    /// Returns false for `InteractionResult.FAIL` (about to break, or Riptide
    /// while dry, or a part denied `trident_throw`) so callers neither dip the
    /// hand nor consume the click.
    fn use_trident(&mut self, stack: &ItemStack) -> bool {
        let throw = super::item_abilities::ability::TRIDENT_THROW;
        if &*stack.item != "minecraft:trident" || !self.stack_can_perform(stack, throw, true) {
            return false;
        }
        let max = self.trident_max_damage(stack);
        let riptide = stack.enchantment_level("minecraft:riptide") > 0;
        if stack.damage >= max.saturating_sub(1) || (riptide && !self.in_water_or_rain()) {
            return false;
        }
        self.start_using_item();
        true
    }

    fn in_water_or_rain(&mut self) -> bool {
        let Some(p) = &self.player else { return false };
        let (x, y, z) = (
            p.position.x.floor() as i32,
            p.position.y.floor() as i32,
            p.position.z.floor() as i32,
        );
        let water = self.res.facts().get(self.world.block(x, y, z)).fluid
            == sunrise_items::blocks::Fluid::Water;
        let (at, top) = (p.position, p.bounds().max_position().y);
        water || self.side_in_rain(at, top)
    }

    /// `instanceof FishingRodItem`.
    fn is_rod(&self, stack: &ItemStack) -> bool {
        self.items()
            .get(&stack.item)
            .is_some_and(|i| i.is("minecraft:fishing_rod"))
    }

    /// `FishingRodCast`: the stack in the arm holding the rod while the hook is out.
    fn cast_rod<'__loan>(&'__loan self) -> Option<&'__loan ItemStack>
    where
        '__sg: '__loan,
    {
        self.hook.as_ref()?;
        if self.main_hand_rod() {
            self.held()
        } else {
            self.offhand.as_ref()
        }
    }

    fn main_hand_rod(&self) -> bool {
        self.held().is_some_and(|s| {
            self.stack_can_perform(
                s,
                super::item_abilities::ability::FISHING_ROD_CAST,
                self.is_rod(s),
            )
        })
    }

    /// The fishing medium a mod says the hook `id` at `position` floats in.
    fn hook_medium(
        &mut self,
        id: u32,
        position: Vec3,
        owner: Option<Vec3>,
        own: bool,
    ) -> Option<mod_api::Answered<mod_api::FishingMedium>> {
        if !mod_api::__private::listened(mod_api::HookId::FishingMedium) {
            return None;
        }
        let epoch = self.native_lifecycle.epoch()?;
        let server = self.is_logical_server();
        let level = self.side_current_level_ref()?;
        let caster = if own {
            self.side_acting_player_ref()
        } else {
            None
        };
        let mut medium = None;
        self.native_fishing_medium(
            level,
            mod_api::EntityRef::new(epoch, id),
            [position.x, position.y, position.z],
            caster,
            owner.map(|o| [o.x, o.y, o.z]),
            &mut medium,
        );
        let mut medium =
            medium.filter(|m| super::fishing_common::inside_medium(position, &m.answer))?;
        if !server {
            medium.answer.loot_table = None;
            medium.answer.lure_ticks = None;
        }
        Some(medium)
    }

    /// `FishingHook.tick`: one tick of movement without the fishing (the server's).
    fn client_hook_step(
        &mut self,
        id: u32,
        owner: Option<Vec3>,
        own: bool,
        (mut position, mut velocity, state, mut ground): (
            Vec3,
            Vec3,
            super::fishing_common::State,
            sunrise_world::ground_support::GroundState,
        ),
        random: &mut impl sunrise_core::random::RandomSource,
    ) -> (
        Vec3,
        Vec3,
        super::fishing_common::State,
        sunrise_world::ground_support::GroundState,
    ) {
        use super::fishing_common::{State, bounds, rebound};
        let b = [
            position.x.floor() as i32,
            position.y.floor() as i32,
            position.z.floor() as i32,
        ];
        let liquid = self.water_height(b[0], b[1], b[2]).map_or(0.0, |w| w.0);
        let surface = if liquid > 0.0 || matches!(state, State::Hooked(_)) {
            None
        } else {
            self.hook_medium(id, position, owner, own)
                .map(|m| m.answer.surface)
        };
        let in_water = liquid > 0.0 || surface.is_some();
        match state {
            State::Flying if in_water => {
                velocity = Vec3::new(velocity.x * 0.3, velocity.y * 0.2, velocity.z * 0.3);
                return (position, velocity, State::Bobbing, ground);
            }
            State::Flying if velocity == Vec3::ZERO && ground.on_ground => {
                return (position, velocity, state, ground);
            }
            State::Flying => {
                if let Some(hit) = self.hook_block_hit(position, position.add_vector(velocity)) {
                    velocity = rebound(velocity, hit.location, position);
                }
            }
            State::Hooked(id) => {
                let hooked = if self.side_remote_owns_bobber_target(id) {
                    self.player
                        .as_ref()
                        .map(|p| (p.position, f64::from(p.dimensions.height)))
                } else {
                    self.mobs
                        .iter()
                        .find(|m| m.id == id && m.alive())
                        .map(|m| (m.position, f64::from(m.height())))
                        .or_else(|| {
                            self.carts
                                .iter()
                                .find(|c| c.id == id)
                                .map(|c| (c.position, super::minecarts::HEIGHT))
                        })
                        .or_else(|| {
                            self.dropped
                                .iter()
                                .find(|i| i.id == id)
                                .map(|i| (i.position, sunrise_sim::entities::ItemEntity::WIDTH))
                        })
                };
                return match hooked {
                    Some((at, height)) => (
                        Vec3::new(at.x, at.y + height * 0.8, at.z),
                        Vec3::ZERO,
                        state,
                        ground,
                    ),
                    None => (position, velocity, State::Flying, ground),
                };
            }
            State::Bobbing => {
                let mut force = match surface {
                    Some(surface) => position.y + velocity.y - surface,
                    None => position.y + velocity.y - f64::from(b[1]) - f64::from(liquid),
                };
                if force.abs() < 0.01 {
                    force += force.signum() * 0.1;
                }
                let r = f64::from(random.next_float());
                velocity = Vec3::new(
                    velocity.x * 0.9,
                    velocity.y - force * r * 0.2,
                    velocity.z * 0.9,
                );
            }
        }
        if !in_water && !ground.on_ground {
            velocity = velocity.add_vector(Vec3::new(0.0, -f64::from(0.03_f32), 0.0));
        }
        let mut scene = sunrise_level::collisions::SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        let mut gate = sunrise_sim::move_gate::armed(
            &self.loop_context,
            id,
            "minecraft:fishing_bobber",
            sunrise_world::entity_move::MoverType::SelfMovement,
            velocity,
        );
        let wanted = gate.delta();
        if let Ok(moved) = sunrise_sim::move_gate::scene_resolve(
            &mut gate,
            &mut scene,
            bounds(position),
            wanted,
            0.0,
            ground.on_ground,
        ) {
            position = Vec3::new(
                position.x + moved.x,
                position.y + moved.y,
                position.z + moved.z,
            );
            self.side_move_bobber_response(id, position, &mut ground, wanted, moved, &mut velocity);
            if state == State::Flying && (ground.on_ground || ground.horizontal_collision) {
                velocity = Vec3::ZERO;
            }
        }
        velocity = Vec3::new(velocity.x * 0.92, velocity.y * 0.92, velocity.z * 0.92);
        (position, velocity, state, ground)
    }

    /// Resource-backed resolution of a mod recipe book declaration slot.
    fn resolve_mod_book_declaration_slot(
        &self,
        slot: &mod_api::RecipeSlotDisplay,
    ) -> Vec<ItemStack> {
        resolve_mod_book_slot_with(self.items(), &self.stations().enchantments, slot)
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> HeldItemsGameExt1<'__sg> for G {}

/// `ProjectileWeaponItem.ARROW_ONLY`: `stack.is(ItemTags.ARROWS)`.
pub fn is_arrow(items: &sunrise_items::items::Items, stack: &ItemStack) -> bool {
    items.item_in_tag("minecraft:arrows", &stack.item)
}

/// `Player.getProjectile`: off hand, main hand, else first arrows in inventory.
pub fn projectile_slot(
    weapon: &str,
    hands: [(usize, Option<&ItemStack>); 2],
    inventory: &[Option<ItemStack>],
    items: &sunrise_items::items::Items,
) -> Option<usize> {
    let held = |s: &ItemStack| {
        is_arrow(items, s)
            || weapon == super::crossbow_common::CROSSBOW
                && &*s.item == super::crossbow_common::FIREWORK_ROCKET
    };
    hands
        .into_iter()
        .find_map(|(slot, s)| s.is_some_and(held).then_some(slot))
        .or_else(|| {
            inventory
                .iter()
                .position(|s| s.as_ref().is_some_and(|s| is_arrow(items, s)))
        })
}

fn dye_slot_stack(target: &ItemStack, dye: usize) -> ItemStack {
    let mut stack = target.clone();
    stack.count = 1;
    let current = stack.data().and_then(|data| data.dyed_color);
    stack.data_mut().dyed_color = Some(sunrise_sim::crafting::apply_dyes(current, &[dye]));
    stack
}

fn resolve_mod_book_slot_with(
    items: &sunrise_items::items::Items,
    enchantments: &sunrise_items::enchant::Enchantments,
    slot: &mod_api::RecipeSlotDisplay,
) -> Vec<ItemStack> {
    use crate::game::api_values::TryIntoEngine;
    use mod_api::RecipeSlotDisplay as Slot;
    match slot {
        Slot::Empty => Vec::new(),
        Slot::Item(id) => items
            .get(id)
            .map(|_| ItemStack::new(id, 1))
            .into_iter()
            .collect(),
        Slot::ItemStack(stack) => stack.try_into_engine(items).ok().into_iter().collect(),
        Slot::AnyFuel => items
            .registry_order
            .iter()
            .filter(|id| items.get(id).is_some_and(|i| i.fuel.is_some()))
            .map(|id| ItemStack::new(id, 1))
            .collect(),
        Slot::Tag(tag) => items
            .item_tag_order
            .get(tag)
            .into_iter()
            .flatten()
            .map(|id| ItemStack::new(id, 1))
            .collect(),
        Slot::Composite(children) => children
            .iter()
            .flat_map(|child| resolve_mod_book_slot_with(items, enchantments, child))
            .collect(),
        Slot::WithRemainder { input, .. } => resolve_mod_book_slot_with(items, enchantments, input),
        Slot::WithAnyPotion(source) => resolve_mod_book_slot_with(items, enchantments, source)
            .into_iter()
            .flat_map(|stack| {
                sunrise_level::brewing::POTIONS
                    .iter()
                    .map(move |p| stack.clone().with_potion(&format!("minecraft:{}", p.id)))
            })
            .collect(),
        Slot::OnlyWithComponent { source, component } => {
            resolve_mod_book_slot_with(items, enchantments, source)
                .into_iter()
                .filter(|stack| stack.carries_component(items.get(&stack.item), component))
                .collect()
        }
        Slot::Dyed { dye, target } => {
            let targets = resolve_mod_book_slot_with(items, enchantments, target);
            resolve_mod_book_slot_with(items, enchantments, dye)
                .iter()
                .map(|stack| {
                    let name = stack
                        .data()
                        .and_then(|data| data.other("minecraft:dye"))
                        .and_then(|tag| match tag {
                            sunrise_core::nbt::Tag::String(value) => Some(value.to_string_lossy()),
                            _ => None,
                        })
                        .or_else(|| items.get(&stack.item).and_then(|info| info.dye.clone()))
                        .unwrap_or_else(|| "white".into());
                    sunrise_level::dyes::DYES
                        .iter()
                        .position(|(id, _)| *id == name)
                        .unwrap_or(0)
                })
                .flat_map(|dye| targets.iter().map(move |stack| dye_slot_stack(stack, dye)))
                .take(16)
                .collect()
        }
        Slot::SmithingTrim {
            base,
            material,
            pattern,
        } => {
            let materials = resolve_mod_book_slot_with(items, enchantments, material);
            let pattern = sunrise_sim::crafting::RecipeTrimPattern::Reference(pattern.clone());
            let bases = resolve_mod_book_slot_with(items, enchantments, base);
            if bases.is_empty() || materials.is_empty() {
                return Vec::new();
            }
            use sunrise_core::random::RandomSource;
            let mut random =
                sunrise_core::random::LegacyRandom::new(slot as *const Slot as usize as i64);
            (0..256)
                .filter_map(|_| {
                    let base = random
                        .next_int_bound(i32::try_from(bases.len()).ok()?)
                        .ok()? as usize;
                    let material = random
                        .next_int_bound(i32::try_from(materials.len()).ok()?)
                        .ok()? as usize;
                    sunrise_sim::stations::apply_trim(
                        enchantments,
                        &bases[base],
                        &materials[material],
                        &pattern,
                    )
                })
                .take(16)
                .collect()
        }
    }
}

std::thread_local! {
    /// Whether a harvest check's hooks are running on this thread.
    static HARVEST_DISPATCH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The break-speed hook of one dig running, given back when dropped.
pub(in crate::game) struct BreakSpeedDispatch {
    pub(in crate::game) outer: bool,
}

impl BreakSpeedDispatch {
    pub(in crate::game) fn enter() -> Self {
        Self {
            outer: BREAK_SPEED_DISPATCH.replace(true),
        }
    }
    pub(in crate::game) fn running() -> bool {
        BREAK_SPEED_DISPATCH.get()
    }
}

impl Drop for BreakSpeedDispatch {
    fn drop(&mut self) {
        BREAK_SPEED_DISPATCH.set(self.outer);
    }
}

std::thread_local! {
    /// Whether a dig's break-speed hook is running on this thread.
    static BREAK_SPEED_DISPATCH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A block Part's `destroy_progress` answer running, given back when dropped (a panic unwinding
/// included). A Part that mines as another block asks the destroy-progress query meanwhile;
/// asking Parts again could run without bound, so the dig math skips them while this is held
/// ([`DestroyProgressDispatch::running`]).
struct DestroyProgressDispatch {
    outer: bool,
}

impl DestroyProgressDispatch {
    fn enter() -> Self {
        Self {
            outer: DESTROY_PROGRESS_DISPATCH.replace(true),
        }
    }
    fn running() -> bool {
        DESTROY_PROGRESS_DISPATCH.get()
    }
}

impl Drop for DestroyProgressDispatch {
    fn drop(&mut self) {
        DESTROY_PROGRESS_DISPATCH.set(self.outer);
    }
}

std::thread_local! {
    /// Whether a block Part's destroy-progress answer is running on this thread.
    static DESTROY_PROGRESS_DISPATCH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The harvest hooks of one `mod_harvest_check` running, given back when dropped.
pub(in crate::game) struct HarvestDispatch {
    pub(in crate::game) outer: bool,
}

impl HarvestDispatch {
    pub(in crate::game) fn enter() -> Self {
        Self {
            outer: HARVEST_DISPATCH.replace(true),
        }
    }
    pub(in crate::game) fn running() -> bool {
        HARVEST_DISPATCH.get()
    }
}

impl Drop for HarvestDispatch {
    fn drop(&mut self) {
        HARVEST_DISPATCH.set(self.outer);
    }
}

/// The item of the stack in `selected` (`Player.tick`'s main-hand item), borrowed: a guest's
/// tick compares it with the last one and clones it only when it changed.
pub fn held_attack_item(inventory: &[Option<ItemStack>], selected: usize) -> Option<&Arc<str>> {
    inventory.get(selected)?.as_ref().map(|s| &s.item)
}

/// `LivingEntity.getItemBlockingWith`'s delay: an item that blocks attacks blocks once it has been
/// used for `BlocksAttacks.blockDelayTicks` (`Math.round(seconds * 20)`) ticks.
pub fn block_delay_passed(component: &sunrise_items::items::BlocksAttacks, ticks: u32) -> bool {
    ticks as i32 >= (component.delay_seconds * 20.0).round() as i32
}

/// The point a `ServerboundInteractPacket` carries: `EntityHitResult.getLocation()
/// - entity.position()`, measured from the entity's feet, as
/// `MultiPlayerGameMode.interact` computes it.
pub fn interact_location(point: Vec3, feet: Vec3) -> [f64; 3] {
    [point.x - feet.x, point.y - feet.y, point.z - feet.z]
}

use crate::game::native_hooks::base::GameHooks as _;

use crate::game::side_ops::items::ItemsSideGameExt as _;

use crate::game::side_ops::player::PlayerSideGameExt as _;

use crate::game::tables::GameItems as _;

use crate::game::game_mode_common::GameModeCommonGameExt1 as _;

use crate::game::item_abilities::ItemAbilitiesGameExt1 as _;

use crate::game::native_components::NativeComponentsGameExt1 as _;

use crate::game::native_hooks::can_harvest_block::GameHooks as _;

use crate::game::native_hooks::destroy_progress::GameHooks as _;

use crate::game::native_hooks::fishing_medium::GameHooks as _;

use crate::game::native_hooks::living_get_projectile::GameHooks as _;

use crate::game::native_hooks::player_break_speed::GameHooks as _;

use crate::game::native_hooks::player_harvest_check::GameHooks as _;

use crate::game::pick_ray::PickRayGameExt1 as _;

use crate::game::player_identity::PlayerIdentityGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;

    fn stack(item: &str) -> ItemStack {
        ItemStack {
            item: item.into(),
            count: 1,
            damage: 0,
            extra: None,
        }
    }

    #[test]
    fn held_attack_item_matches_the_cloned_stack() {
        let mut inventory: Vec<Option<ItemStack>> = vec![None; 36];
        inventory[0] = Some(stack("minecraft:stone"));
        inventory[3] = Some(stack("minecraft:diamond_sword"));
        for selected in [0, 1, 3, 35, 36, 1000] {
            // The clone the tick used to make, as the reference.
            let cloned = inventory.get(selected).cloned().flatten().map(|s| s.item);
            assert_eq!(held_attack_item(&inventory, selected).cloned(), cloned);
        }
        assert_eq!(held_attack_item(&[], 0), None);
    }
}
