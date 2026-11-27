//! Shields and other `blocks_attacks` items: raising them (`ShieldItem.use`,
//! from either hand), `LivingEntity.getItemBlockingWith` and
//! `applyItemBlocking` inputs, and what follows a block: item damage,
//! the block sound, attacker knockback (`blockedByItem`) and axes disabling
//! the shield (`Player.blockUsingItem`, `BlocksAttacks.disable`).
use crate::game::ServerGame;
use sunrise_core::random::RandomSource;
use sunrise_gameplay_common::game::held_items::block_delay_passed;
#[allow(unused_imports)]
use sunrise_items::items::ItemStack;
use sunrise_sim::health::Attack;
#[allow(unused_imports)]
use sunrise_sim::health::{BlockOutcome, Blocking, HurtSource};
use sunrise_world::blocking_rules::{BlockingRules, DamageReduction, ItemDamageFunction};

/// `Inventory.SLOT_OFFHAND`: the `using` slot of an off-hand item.
pub use super::held_items::OFFHAND_SLOT;

impl ServerGame<'_> {
    /// `AbstractArrow.onHitEntity` on the player: a piercing arrow passes item blocking, whoever
    /// shot it, because `applyItemBlocking` reads the direct entity's pierce level.
    pub(in crate::game::server_side) fn arrow_hits_player(
        &mut self,
        amount: f32,
        p: &sunrise_sim::projectiles::Projectile,
    ) -> bool {
        self.server.combat.0.piercing_hit = p.arrow.as_ref().is_some_and(|a| a.pierce > 0);
        self.mob_hits_player(
            amount,
            [p.previous.x, p.previous.y, p.previous.z],
            "minecraft:arrow",
        )
    }
    /// `ShieldItem.use` (ItemUtils.startUsingInstantly) for the hand at `slot`;
    /// false when that hand holds no blocking item or it is on cooldown.
    pub fn raise_shield(&mut self, slot: usize) -> bool {
        let Some(stack) = self.common.stack_in(slot) else {
            return false;
        };
        if self.blocks_attacks(stack).is_none() {
            return false;
        }
        if self.server.shield_cooldown == 0 {
            self.start_using_item();
        }
        true
    }

    /// `LivingEntity.getItemBlockingWith`: the blocking component of the active
    /// item after its use delay, shared by damage and the player's body turn.
    pub(in crate::game) fn player_blocking_component(
        &self,
    ) -> Option<std::borrow::Cow<'_, sunrise_items::items::BlocksAttacks>> {
        let (slot, ticks) = self.common.using?;
        let component = self.blocks_attacks(self.common.stack_in(slot)?)?;
        block_delay_passed(&component, ticks).then_some(component)
    }
    /// `LivingEntity.getItemBlockingWith` and the inputs of `applyItemBlocking`
    /// for an attack from `from` (`DamageSource.getSourcePosition`).
    pub fn player_blocking(
        &self,
        from: Option<[f64; 3]>,
        damage_type: &str,
        pierce: bool,
    ) -> Option<Blocking> {
        let component = self.player_blocking_component()?;
        let tags = &self.stations().enchantments.damage_tags;
        let matches = |spec: &str| match spec.strip_prefix('#') {
            Some(tag) => tags.get(tag).is_some_and(|t| t.contains(damage_type)),
            None => spec == damage_type,
        };
        let p = self.common.player.as_ref()?;
        // LivingEntity.applyItemBlocking: Math.acos of the flattened direction against
        // calculateViewVector(0, yHeadRot), or (float)Math.PI with no source position.
        let angle = match from {
            Some(src) => sunrise_world::view_direction::blocking_angle(
                sunrise_core::geometry::Vec3::new(src[0], src[1], src[2]),
                p.position,
                self.common.yaw,
            ),
            None => sunrise_world::view_direction::BLOCKING_FALLBACK_ANGLE,
        };
        let (threshold, base, factor) = component.item_damage;
        Some(Blocking {
            rules: BlockingRules {
                delay_seconds: component.delay_seconds,
                disable_scale: component.disable_scale,
                reductions: component
                    .reductions
                    .iter()
                    .map(|r| DamageReduction {
                        horizontal_angle: r.horizontal_angle,
                        types: r.types.as_ref().map(|t| t.iter().any(|s| matches(s))),
                        base: r.base,
                        factor: r.factor,
                    })
                    .collect(),
                item_damage: ItemDamageFunction {
                    threshold,
                    base,
                    factor,
                },
            },
            bypassed: component.bypassed_by.as_deref().is_some_and(matches),
            pierce,
            angle,
        })
    }
    /// After a hurt with the shield raised: `hurtBlockingItem`, `onBlocked`
    /// and, for melee from a mob, `blockUsingItem`. `source` is the hit's.
    pub fn after_block(
        &mut self,
        outcome: BlockOutcome,
        (attacker, projectile): (Option<u32>, bool),
        source: HurtSource<'_>,
        attack: &Attack,
    ) {
        let Some((slot, _)) = self.common.using else {
            return;
        };
        let Some(stack) = self.common.stack_in(slot).cloned() else {
            return;
        };
        let Some(component) = self
            .blocks_attacks(&stack)
            .map(std::borrow::Cow::into_owned)
        else {
            return;
        };
        let at = self
            .common
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        if outcome.sound
            && let Some(sound) = &component.block_sound
        {
            let pitch = 0.8 + self.common.world.random.next_float() * 0.4;
            if let Some(at) = at {
                self.server_play_sound(sound, at, 1.0, pitch);
            }
        }
        // BlocksAttacks.hurtBlockingItem counts use even when nothing is blocked.
        if !outcome.blocking_item_used {
            return;
        }
        // `ItemStack.hurtAndBreak` does nothing for `hasInfiniteMaterials`, its
        // `ITEM_USED` award included: a creative player's shield counts no use.
        if self.common.is_logical_server() && !self.common.creative {
            self.award_stat("used", &stack.item, 1);
        }
        // Component wear can apply even when no damage was blocked. A mod's wear
        // override applies only when the hook leaves the hit blocked.
        let item_damage = outcome.shield_damage;
        if item_damage > 0 && !self.common.creative {
            let (updated, broken) = self.damage_stack(&stack, item_damage, true);
            if self.common.stack_in(slot) == Some(&stack) {
                *self.common.stack_in_mut(slot) = updated;
                if let Some(broken) = broken {
                    self.common.using = None;
                    // ItemStack.hurtAndBreak: onEquippedItemBroken for the hand it was used from.
                    let hand = if slot == OFFHAND_SLOT {
                        sunrise_core::equipment_slot::EquipmentSlot::OffHand
                    } else {
                        sunrise_core::equipment_slot::EquipmentSlot::MainHand
                    };
                    self.player_equipment_broke(hand, &broken);
                    self.player_item_destroyed(&stack, self.common.item_slot_hand(slot));
                    self.award_stat("broken", &broken.item, 1);
                    return;
                }
            }
        }
        // LivingEntity.blockUsingItem: only once something was stopped.
        if outcome.blocked <= 0.0 || projectile {
            return;
        }
        let Some(id) = attacker else {
            // Another player's melee hit has no mob behind it.
            if let Some((from, disable)) = self.server.combat.0.player_attacker {
                let fully = outcome.fully_blocked;
                let attacker = ([from[0], from[2]], disable);
                self.blocked_by_attacker(
                    &component,
                    attacker,
                    fully,
                    at,
                    source,
                    attack,
                    outcome.blocked,
                );
            }
            return;
        };
        let fully = outcome.fully_blocked;
        let Some(m) = self.common.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let (mx, mz) = (m.position.x, m.position.z);
        // LivingEntity.getSecondsToDisableBlocking: the held weapon's value;
        // Warden overrides it with a flat 5 seconds.
        let disable = if m.species() == sunrise_sim::mobs::Species::Warden {
            5.0
        } else {
            m.extra
                .main_hand_id()
                .and_then(|item| self.items().get(item))
                .map_or(0.0, |i| i.disable_blocking_seconds)
        };
        // Ravager.blockedByItem: stunned half the time (the defender pushed
        // apart), else the defender flung away; no roar in progress.
        if m.species() == sunrise_sim::mobs::Species::Ravager {
            let mut events = Vec::new();
            let roaring = m.extra.anim_ticks != 0;
            let stunned = self
                .common
                .mobs
                .iter_mut()
                .find(|m| m.id == id)
                .is_some_and(|m| sunrise_sim::mobs::ravager::blocked(m, &mut events));
            if !roaring && let Some(p) = self.common.player.as_mut() {
                let (xd, zd) = (p.position.x - mx, p.position.z - mz);
                let push = if stunned {
                    // Entity.push(entity): apart by 0.05 per block of overlap.
                    let d = (xd * xd + zd * zd).sqrt().max(0.01);
                    let k = (1.0 / d).min(1.0) * 0.05;
                    sunrise_core::geometry::Vec3::new(xd / d * k, 0.0, zd / d * k)
                } else {
                    let dd = (xd * xd + zd * zd).max(0.001);
                    sunrise_core::geometry::Vec3::new(xd / dd * 4.0, 0.2, zd / dd * 4.0)
                };
                p.velocity = p.velocity.add_vector(push);
            }
            let _ = self.handle_mob_events(events);
            return;
        }
        self.blocked_by_attacker(
            &component,
            ([mx, mz], disable),
            fully,
            at,
            source,
            attack,
            outcome.blocked,
        );
    }

    /// What a blocked melee hit does to the defender once the shield took
    /// it: `LivingEntity.blockedByItem` knocks them back unless fully
    /// blocked, then `Player.blockUsingItem` disables the shield. `attacker`
    /// is where the attacker stands (x, z) and its seconds of disabling.
    fn blocked_by_attacker(
        &mut self,
        component: &sunrise_items::items::BlocksAttacks,
        ([mx, mz], disable): ([f64; 2], f32),
        fully: bool,
        at: Option<[f64; 3]>,
        source: HurtSource<'_>,
        attack: &Attack,
        damage: f32,
    ) {
        // LivingEntity.blockedByItem: defender.knockback(0.5, defender - attacker,
        // source), which moves the defender along -(xd, zd).
        if !fully && let Some(p) = self.common.player.as_ref() {
            let (xd, zd) = (p.position.x - mx, p.position.z - mz);
            self.knock_back_player(0.5, xd, zd, source, attack, damage);
        }
        // BlocksAttacks.disable: item cooldown and stopUsingItem.
        let ticks = disable * component.disable_scale;
        let ticks = if ticks > 0.0 {
            (ticks * 20.0).round() as u32
        } else {
            0
        };
        if ticks > 0 {
            if let (Some(player), Some(name)) = (
                self.server.lan_acting_uuid,
                self.server.lan_acting_name.clone(),
            ) {
                self.native_player_shield_disable(player, &name, ticks);
            }
            let used = self
                .common
                .using
                .and_then(|(slot, _)| self.common.stack_in(slot))
                .cloned();
            let item = used.as_ref().map(|stack| stack.item.clone());
            let mut cancel = false;
            if let (Some(player), Some(name), Some(item)) = (
                self.server.lan_acting_uuid,
                self.server.lan_acting_name.clone(),
                item,
            ) {
                self.native_player_item_cooldown(player, &name, &item, ticks as i32, &mut cancel);
            }
            if !cancel {
                self.server.shield_cooldown = ticks;
                // ItemCooldowns.addCooldown(stack, ticks) on the shield's group: a guest is
                // sent it (ServerItemCooldowns), which keeps the shield down on their screen,
                // and the 0 that ends it.
                if let Some(stack) = &used {
                    let default = self
                        .items()
                        .get(&stack.item)
                        .and_then(|i| i.use_cooldown.clone());
                    let group = sunrise_items::cooldowns::group(stack, default.as_ref());
                    self.add_cooldown(group, ticks as i32);
                }
            }
            self.stop_active_item_use();
            if let Some(sound) = &component.disabled_sound {
                let pitch = 0.8 + self.common.world.random.next_float() * 0.4;
                if let Some(at) = at {
                    self.server_play_sound(sound, at, 0.8, pitch);
                }
            }
        }
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;
