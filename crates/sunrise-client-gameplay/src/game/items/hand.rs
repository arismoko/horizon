//! The local player's hand: clicks and held attack through
//! `sunrise_client::first_person` (`Minecraft.startAttack` /
//! `continueAttack`, `MultiPlayerGameMode` block breaking), the arm swing
//! (`LivingEntity.SwingState`) and the first-person equip heights
//! (`FirstPersonHandsAndItems`) with the item swap ticker they read.
#[cfg(test)]
use super::held_items::in_attack_range;
use super::held_items::{attack_range_contains, attack_reach};
use super::minecarts::CartKind;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_items::items::ItemStack;
use sunrise_sim::attack_strength::HeldStack;
use sunrise_sim::first_person_sim::{Hand, SwingAnimation, swing_duration};

/// The off hand's place in `visible_hands` (the main hand uses its hotbar slot).
pub(in crate::game) const OFFHAND: usize = usize::MAX;

pub(in crate::game::client_side) fn v3(v: [f64; 3]) -> Vec3 {
    Vec3::new(v[0], v[1], v[2])
}

/// `LocalPlayer.raycastHitResult` with an item that has an attack range
/// (`AttackRange.getClosesetHit`): the distance from the eye to the nearest
/// box hit along the item's own `reach`, which goes past the plain pick's
/// 3 blocks (a spear's 4.5). A miss falls back to the plain pick (`None`).
pub(in crate::game::client_side) fn reach_pick(
    boxes: &[(usize, Aabb)],
    eye: Vec3,
    look: Vec3,
    movement: Vec3,
    reach: (f32, f32, f32),
    clip: impl Fn(Vec3, Vec3) -> Option<Vec3>,
) -> Option<f64> {
    super::spear::hits_in_reach(boxes, eye, look, movement, reach, clip)
        .into_iter()
        .map(|(_, at)| eye.distance_squared(at).sqrt())
        .reduce(f64::min)
}

/// `FirstPersonHandsAndItems.shouldInstantlyReplaceVisibleItem`'s
/// `ItemStack.matchesIgnoringComponents(visible, held,
/// DataComponentType::ignoreSwapAnimation)` for one hand (every item plays
/// the swap animation, `ItemModelResolver.shouldPlaySwapAnimation`): the same
/// stack object, or an equal one, whose damage (the one component built
/// with `ignoreSwapAnimation`) may differ.
///
/// A stack that stays in its slot with the same item is the same object,
/// whatever its count or damage (vanilla shrinks, grows and damages it in
/// place), unless the slot was `replaced` by one the hand doesn't show yet:
/// swapping the hands (`setItemInHand`, and the server's slot updates that
/// answer it) puts another object in both (#2866).
pub(in crate::game::client_side) fn shows_held_stack(
    visible: &(usize, Option<ItemStack>),
    held: &(usize, Option<ItemStack>),
    replaced: bool,
) -> bool {
    let same_object = visible.0 == held.0 && !replaced;
    match (&visible.1, &held.1) {
        (Some(a), Some(b)) if same_object => a.item == b.item,
        (a, b) => matches_for_swap(a.as_ref(), b.as_ref()),
    }
}

/// `ItemStack.matchesIgnoringComponents(a, b,
/// DataComponentType::ignoreSwapAnimation)`: the same count, item and
/// components but damage.
pub(in crate::game) fn matches_for_swap(a: Option<&ItemStack>, b: Option<&ItemStack>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.count == b.count && a.item == b.item && a.extra == b.extra,
        _ => false,
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// A slot of the inventory (`OFFHAND_SLOT` for the off hand) was set to
    /// a new stack: the hand holding it compares it as another `ItemStack`
    /// (`shows_held_stack`) until the hand shows the held stack again.
    pub fn hand_stack_replaced(&mut self, slot: usize) {
        if slot == self.common.selected {
            self.client.hands_replaced[0] = true;
        }
        if slot == super::held_items::OFFHAND_SLOT {
            self.client.hands_replaced[1] = true;
        }
    }
    /// Opening a screen stops breaking before the next game tick (which may pause).
    pub(in crate::game) fn stop_mining(&mut self) {
        if self.common.clicks.destroying.active {
            let pos = self.common.clicks.destroying.pos;
            let state = self.common.world.block(pos[0], pos[1], pos[2]);
            self.tutorial_destroy(state, -1.0);
        }
        self.common.clicks.destroying = Default::default();
        self.request_remote_mining_stop();
        self.set_client_mining(None);
    }
    /// Returns false when the interaction policy or item vetoes the swing and its packet.
    pub(in crate::game) fn client_swing_hand(
        &mut self,
        hand: Hand,
        animation: SwingAnimation,
    ) -> bool {
        if self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| !r.interaction_swing[usize::from(hand == Hand::Off)])
        {
            return false;
        }
        if self.native_entity_swing_event(hand) {
            return false;
        }
        let effects = &self.common.potions.effects;
        let dig_speed = effects
            .amplifier("haste")
            .max(effects.amplifier("conduit_power"));
        let fatigue = effects.amplifier("mining_fatigue");
        let duration = swing_duration(animation, dig_speed, fatigue);
        if self.common.swing.start_if_able(hand, animation, duration) {
            self.common.swings_started += 1;
            if let Some(r) = self.client.remote.as_ref()
                && r.session.is_recording()
            {
                let pkt = sunrise_protocol::packets::play::Clientbound::SwingAnimation(
                    sunrise_protocol::packets::play::SwingAnimation {
                        id: r.entity_id,
                        hand: if hand == Hand::Off { 1 } else { 0 },
                        kind: animation.kind as i32,
                        duration,
                    },
                );
                r.session.record_clientbound(&pkt);
            }
        }
        true
    }
    /// The window tick already picked before key handling; simulation keeps its own ray.
    pub fn pick_tick_targets(&mut self) {
        if self.common.native_window() {
            return;
        }
        self.client.tick_hit.set(None);
        self.client.tick_mob = None;
        match self.compute_crosshair_decision() {
            super::pick_entity::CrosshairDecision::Block(hit) => {
                self.client.tick_hit.set(Some(Some(hit)));
                self.client.tick_mob = Some(None);
            }
            super::pick_entity::CrosshairDecision::Entity(hit) => {
                self.client.tick_hit.set(Some(None));
                let mob = self
                    .common
                    .mob_index
                    .find(&self.common.mobs, hit.id)
                    .map(|i| {
                        let feet = self.common.mobs[i].position;
                        (
                            i,
                            hit.distance_sq.sqrt(),
                            [
                                hit.point[0] - feet.x,
                                hit.point[1] - feet.y,
                                hit.point[2] - feet.z,
                            ],
                        )
                    });
                self.client.tick_mob = Some(mob);
            }
            super::pick_entity::CrosshairDecision::Miss => {
                self.client.tick_hit.set(Some(None));
                self.client.tick_mob = Some(None);
            }
        }
    }
    /// Releases the non-window simulation's tick-local pick caches.
    pub fn release_tick_targets(&mut self) {
        self.client.tick_hit.set(None);
        self.client.tick_mob = None;
    }
    /// `handleKeybinds`' number keys: keys 1 to 9 in that order, one click of each
    /// (`keyHotbarSlots[i].consumeClick`), so the highest key clicked this tick wins and a
    /// key's further clicks wait for the next ticks. A creative player holding the load or
    /// save toolbar key loads or saves that toolbar instead (load first when both are held).
    pub(in crate::game::client_side) fn select_queued_hotbar(&mut self) {
        const HOTBAR: [&str; 9] = [
            "key.hotbar.1",
            "key.hotbar.2",
            "key.hotbar.3",
            "key.hotbar.4",
            "key.hotbar.5",
            "key.hotbar.6",
            "key.hotbar.7",
            "key.hotbar.8",
            "key.hotbar.9",
        ];
        for (slot, name) in HOTBAR.into_iter().enumerate() {
            if !self.consume_mapping_click(name) {
                continue;
            }
            if self.own_player_is_spectator() {
                self.spectator_hotbar_key(slot as i32);
                continue;
            }
            let load = self.common.creative && self.mapped_key_down("key.loadToolbarActivator");
            if load || (self.common.creative && self.mapped_key_down("key.saveToolbarActivator")) {
                self.hotbar_load_or_save(slot, load);
            } else {
                self.common.selected = slot;
            }
        }
    }
    pub fn client_pick_entity_reach(&self, direction: [f64; 3]) -> f64 {
        self.common
            .active_attack_range()
            .map_or(self.common.entity_reach, |range| {
                (f64::from(attack_reach(&range, self.own_player_is_creative()).1)
                    + self.known_player_movement().dot(v3(direction)).max(0.0))
                .max(self.common.entity_reach)
            })
    }

    /// `AttackRange.getClosesetHit` plus `LocalPlayer.filterHitResult` on block hits:
    /// preserves entity hits, in-range block hits, and misses separately so block hits
    /// do not fall back to the plain `LocalPlayer.pick` ray.
    pub fn attack_range_pick_result(
        &self,
        range: &serde_json::Value,
    ) -> super::pick_entity::AttackRangePickResult {
        #[cfg(feature = "client")]
        use super::pick_accumulator::PickCandidate;
        use super::pick_entity::AttackRangePickResult;

        let Some((partial_eye_arr, look_arr)) = self.aim() else {
            return AttackRangePickResult::Miss;
        };
        let partial_eye = v3(partial_eye_arr);
        let look = v3(look_arr);
        let eye = self.camera_eye_position().map(v3).unwrap_or(partial_eye);
        let (min, max, margin) = attack_reach(range, self.own_player_is_creative());
        let movement = self.known_player_movement();
        let from = eye.add_vector(look.scale(f64::from(min)));
        let max_len = f64::from(max) + movement.dot(look).max(0.0);
        let mut to = eye.add_vector(look.scale(max_len));

        let filter_block = |hit: super::Hit| {
            let d2 =
                partial_eye.distance_squared(Vec3::new(hit.point[0], hit.point[1], hit.point[2]));
            if d2 < self.common.block_reach * self.common.block_reach {
                AttackRangePickResult::Block(hit)
            } else {
                AttackRangePickResult::Miss
            }
        };

        let block_hit = (max_len > 0.0)
            .then(|| self.clip_ray([eye.x, eye.y, eye.z], [look.x, look.y, look.z], max_len))
            .flatten();
        if let Some(hit) = block_hit {
            to = Vec3::new(hit.point[0], hit.point[1], hit.point[2]);
            if eye.distance_squared(to) < eye.distance_squared(from) {
                return filter_block(hit);
            }
        }

        let clip = |a: Vec3, b: Vec3| {
            let length = a.distance_squared(b).sqrt();
            if length <= 0.0 {
                return None;
            }
            let d = b.subtract(a).scale(1.0 / length);
            self.clip_ray([a.x, a.y, a.z], [d.x, d.y, d.z], length)
                .map(|hit| Vec3::new(hit.point[0], hit.point[1], hit.point[2]))
        };

        let half = f64::from(margin) * 0.5;
        let search_area = Aabb::new(
            Vec3::new(from.x - half, from.y - half, from.z - half),
            Vec3::new(from.x + half, from.y + half, from.z + half),
        )
        .expand_towards(to.subtract(from))
        .inflate(Vec3::new(1.0, 1.0, 1.0));
        let grow = Vec3::new(f64::from(margin), f64::from(margin), f64::from(margin));

        let mut nearest: Option<PickCandidate> = None;
        // AttackRange uses ProjectileUtil.getManyEntityHitResult over CAN_BE_PICKED:
        // full pickable population, no entity pickRadius inflation, no root-vehicle exclusion.
        self.for_each_pickable_in_bounds(search_area, |id, raw_bb, _radius| {
            let inside = raw_bb.contains(from);
            let hit_point = if inside {
                Some(from)
            } else if let Some(at) = raw_bb.clip(from, to) {
                Some(at)
            } else if margin > 0.0 {
                raw_bb.inflate(grow).clip(from, to).and_then(|outside| {
                    let centre = raw_bb.center();
                    let toward = clip(outside, centre).unwrap_or(centre);
                    raw_bb.clip(outside, toward)
                })
            } else {
                None
            };
            if let Some(at) = hit_point {
                let distance_sq = partial_eye.distance_squared(at);
                if nearest.is_none_or(|best| distance_sq < best.distance_sq) {
                    nearest = Some(PickCandidate {
                        id,
                        point: [at.x, at.y, at.z],
                        distance_sq,
                        inside,
                    });
                }
            }
        });

        if let Some(hit) = nearest {
            AttackRangePickResult::Entity(hit)
        } else if let Some(hit) = block_hit {
            filter_block(hit)
        } else {
            AttackRangePickResult::Miss
        }
    }
    /// Whether an attack would hit an entity rather than the block behind it.
    pub fn entity_targeted(&self) -> bool {
        if self.uses_stored_pick() {
            return matches!(
                self.crosshair_target(),
                Some(mod_api::CrosshairTarget::Entity { .. })
            );
        }
        self.crosshair_entity_hit().is_some()
    }
    /// `LocalPlayer.isHandsBusy`: paddling a boat this player controls
    /// (`getControlledVehicle`: its first passenger, on a server's client).
    pub(in crate::game::client_side) fn hands_busy(&self) -> bool {
        self.common.riding.is_some_and(|id| self.controls_boat(id))
            && [
                winit::keyboard::KeyCode::KeyW,
                winit::keyboard::KeyCode::KeyS,
                winit::keyboard::KeyCode::KeyA,
                winit::keyboard::KeyCode::KeyD,
            ]
            .iter()
            .any(|k| self.movement_key_held(*k))
    }
    /// `LocalPlayer.tick`: the swing (`LivingEntity.baseTick`), the attack
    /// strength and item swap tickers (`Player.tick`), then
    /// `FirstPersonHandsAndItems.tick`.
    pub fn tick_hand(&mut self) {
        // LocalPlayer.applyInput (the camera is the player's own).
        if self.common.player.is_some() {
            self.client
                .sway
                .tick_wrapped(self.common.pitch, self.common.yaw);
        }
        self.common.swing.tick();
        let main = self.common.inventory[self.common.selected].clone();
        // Player.tick: a different item resets the attack strength; any change
        // of the stack (`!ItemStack.matches`, e.g. another Efficiency level on
        // the same tool) is what detectEquipmentUpdates gives the player its
        // modifiers for.
        let held = main.as_ref().map(|s| HeldStack {
            item: s.item.to_string(),
            rest: format!("{} {} {:?}", s.count, s.damage, s.extra),
        });
        let changed = self.client.hand_strength.main_hand_changed(&held);
        let _ = self.client.hand_strength.tick(held);
        if changed {
            self.apply_equipment();
        }
        let next = [
            (self.common.selected, main),
            (OFFHAND, self.common.offhand.clone()),
        ];
        let mut changed = [false; 2];
        for h in 0..2 {
            let replaced = self.client.hands_replaced[h];
            let mut same = shows_held_stack(&self.client.visible_hands[h], &next[h], replaced);
            self.reequip_part(h, &next[h], &mut same);
            if same {
                self.client.visible_hands[h] = next[h].clone();
                self.client.hands_replaced[h] = false;
            }
            changed[h] = !same;
        }
        let speed = self.main_hand_attack_speed();
        let replace = self.client.heights.tick_with(
            self.hands_busy(),
            self.client.hand_strength.swap_scale(1.0, speed),
            changed,
            self.pvp_legacy_look(),
        );
        for h in 0..2 {
            if replace[h] {
                self.client.visible_hands[h] = next[h].clone();
                self.client.hands_replaced[h] = false;
            }
        }
    }

    /// `IItemExtension.shouldCauseReequipAnimation` for hand `h` (0 main, 1
    /// off), asked while the stack it shows differs from `held` by slot or
    /// by value and `held` isn't empty: `same` (shown at once) is vanilla's
    /// answer, and the parts of the held item may turn it either way.
    pub(in crate::game::client_side) fn reequip_part(
        &self,
        h: usize,
        held: &(usize, Option<ItemStack>),
        same: &mut bool,
    ) {
        let Some(stack) = &held.1 else {
            return;
        };
        let visible = &self.client.visible_hands[h];
        if visible.0 == held.0 && visible.1.as_ref() == Some(stack) {
            return;
        }
        if !mod_api::__private::item_listens(
            mod_api::HookId::ShouldCauseReequipAnimation,
            &stack.item,
        ) {
            return;
        }
        let hand = if h == 0 { Hand::Main } else { Hand::Off };
        let slot_changed = h == 0 && visible.0 != held.0;
        let mut reequip = !*same;
        self.native_should_cause_reequip_animation(
            stack,
            visible.1.as_ref(),
            hand,
            slot_changed,
            &mut reequip,
        );
        *same = !reequip;
    }

    /// A replay owns its held-item and equip animation state. Reset it when a
    /// standalone replay closes so a later world cannot inherit lowered hands
    /// or the replay player's visible stacks.
    pub(in crate::game::client_side) fn reset_studio_hand_state(&mut self) {
        let main = self
            .common
            .inventory
            .get(self.common.selected)
            .cloned()
            .flatten();
        self.client.visible_hands = [
            (self.common.selected, main),
            (OFFHAND, self.common.offhand.clone()),
        ];
        self.client.hands_replaced = [false; 2];
        self.client.heights = sunrise_client::first_person::HandHeights {
            main: 1.0,
            old_main: 1.0,
            off: 1.0,
            old_off: 1.0,
        };
        self.common.swing = Default::default();
        self.client.sway = Default::default();
    }

    /// `Minecraft.crosshairPickEntity` when it is a `LivingEntity`: whether
    /// it is alive (the nearest entity under the crosshair, before any block).
    pub fn crosshair_living(&self) -> Option<bool> {
        if let Some(hit) = self.cached_entity_pick() {
            let (entity, distance, _) = hit?;
            if !self.common.mobs.iter().any(|mob| mob.id == entity.id())
                && !self
                    .common
                    .armor_stands
                    .iter()
                    .any(|stand| stand.id == entity.id())
            {
                return None;
            }
            if self.common.active_attack_range().is_some_and(|range| {
                !attack_range_contains(&range, distance, self.own_player_is_creative())
            }) {
                return None;
            }
            return Some(true);
        }
        let active = self
            .common
            .using
            .and_then(|(slot, _)| self.common.inventory.get(slot).and_then(Option::as_ref))
            .or_else(|| self.common.held());
        let range = active.and_then(|stack| self.common.attack_range_component(stack));
        let spear = range.as_ref().and_then(|r| {
            let eye = self.common.eye_position()?;
            let look = super::view_vector(self.common.yaw, self.common.pitch);
            let reach = attack_reach(r, self.own_player_is_creative());
            let boxes = self.common.hittable_mob_boxes();
            let clip = |from: Vec3, to: Vec3| {
                let length = from.distance_squared(to).sqrt();
                let d = to.subtract(from).scale(1.0 / length);
                (length > 0.0)
                    .then(|| self.clip_ray([from.x, from.y, from.z], [d.x, d.y, d.z], length))
                    .flatten()
                    .map(|hit| Vec3::new(hit.point[0], hit.point[1], hit.point[2]))
            };
            let movement = self.known_player_movement();
            reach_pick(&boxes, v3(eye), v3(look), movement, reach, clip)
        });
        let living = match spear {
            Some(distance) => distance,
            None => {
                let hit = self.crosshair_entity_hit()?;
                let uid = u32::try_from(hit.id).ok()?;
                if !self.common.mobs.iter().any(|m| m.id == uid)
                    && !self.common.armor_stands.iter().any(|a| a.id == uid)
                {
                    return None;
                }
                hit.distance
            }
        };
        // Hud.extractCrosshair: with an attack range, the hit must be in it.
        if let Some(range) = &range
            && !attack_range_contains(range, living, self.own_player_is_creative())
        {
            return None;
        }
        // Only alive mobs are hit, so a picked living entity is alive.
        Some(true)
    }
    /// `Hud.canRenderCrosshairForSpectator`: MenuProvider entity or spectator-interactable block.
    pub fn spectator_crosshair(&self) -> bool {
        if let Some(hit) = self.crosshair_entity_hit() {
            return self.common.carts.iter().any(|c| {
                c.id as i32 == hit.id && matches!(c.kind, CartKind::Chest | CartKind::Hopper)
            }) || self
                .common
                .boats
                .iter()
                .any(|b| b.id as i32 == hit.id && b.chest);
        }
        self.pick()
            .is_some_and(|hit| self.block_interactable_for_spectator(&hit))
    }
    /// `BlockBehaviour.showAsInteractableInSpectatorMode`: a block with a menu, or a portal.
    pub(in crate::game::client_side) fn block_interactable_for_spectator(
        &self,
        hit: &super::Hit,
    ) -> bool {
        let block = self.common.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        matches!(
            &*self.res.facts().get(block).name,
            "minecraft:nether_portal" | "minecraft:end_portal" | "minecraft:end_gateway"
        ) || self.block_opens_menu(hit)
    }
    /// A lectern with no book (`LecternBlock.useWithoutItem` consumes the click and does nothing).
    pub(in crate::game::client_side) fn block_is_empty_lectern(&self, hit: &super::Hit) -> bool {
        let state = self.common.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        &*self.common.res.facts().get(state).name == "minecraft:lectern"
            && self
                .common
                .pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get("has_book"))
                .is_none_or(|v| v != "true")
    }
    /// `BlockState.getMenuProvider`: the block's use opens a screen.
    pub(in crate::game::client_side) fn block_opens_menu(&self, hit: &super::Hit) -> bool {
        if self.block_is_empty_lectern(hit) {
            return false;
        }
        let id = &*self
            .common
            .res
            .facts()
            .get(self.common.world.block(hit.pos[0], hit.pos[1], hit.pos[2]))
            .name;
        let name = id.trim_start_matches("minecraft:");
        match name {
            // BlockBehaviour.getMenuProvider.
            "crafting_table" | "enchanting_table" | "anvil" | "chipped_anvil" | "damaged_anvil"
            | "stonecutter" | "loom" | "cartography_table" | "grindstone" | "smithing_table"
            | "barrel" | "furnace" | "blast_furnace" | "smoker" | "brewing_stand" | "hopper"
            | "dispenser" | "dropper" | "crafter" | "lectern" | "beacon" => true,
            // ChestBlock.getMenuProvider: none while a redstone conductor
            // sits on top of either half (a sitting cat aside). Copper
            // chests are `ChestBlock`s too.
            _ if sunrise_level::containers::is_chest_block(id) => {
                let key = (hit.pos[0], hit.pos[1], hit.pos[2]);
                let blocked = |p: (i32, i32, i32)| {
                    let above = self.common.world.block(p.0, p.1 + 1, p.2);
                    self.common.res.behaviours().facts(above).flags
                        & sunrise_world::level::data::state_flags::CONDUCTOR
                        != 0
                };
                !blocked(key)
                    && self
                        .common
                        .chest_other_half(key)
                        .is_none_or(|o| !blocked(o))
            }
            _ => name.ends_with("shulker_box"),
        }
    }
    /// `evaluateWhichHandsToRender`: which of the main and off hand show.
    pub(in crate::game::client_side) fn hands_to_render(&self) -> [bool; 2] {
        let held = |slot| {
            self.common
                .stack_in(slot)
                .map(|s: &ItemStack| (&*s.item, s.data().is_some_and(|d| !d.charged.is_empty())))
        };
        let using = self.common.used_hand().zip(
            self.common
                .using
                .and_then(|(slot, _)| self.common.stack_in(slot))
                .map(|s| &*s.item),
        );
        sunrise_client::first_person::hands_to_render(
            [
                held(self.common.selected),
                held(super::held_items::OFFHAND_SLOT),
            ],
            using,
        )
    }
    /// The visible main- and off-hand items.
    pub(in crate::game::client_side) fn visible_hand_items(&self) -> [Option<ItemStack>; 2] {
        [
            self.client.visible_hands[0].1.clone(),
            self.client.visible_hands[1].1.clone(),
        ]
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::item_attack::ItemAttackGameExt2 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;

    /// A zombie-sized box whose near face is `distance` blocks ahead of an
    /// eye at the origin looking along +z.
    fn mob_ahead(distance: f64) -> Vec<(usize, Aabb)> {
        let min = Vec3::new(-0.3, -1.0, distance);
        vec![(0, Aabb::new(min, min.add_vector(Vec3::new(0.6, 1.95, 0.6))))]
    }

    #[test]
    fn a_spear_picks_a_mob_past_the_plain_reach() {
        // #3209: a mob 4 blocks ahead is out of the plain pick's 3 blocks but
        // within a spear's attack range (min 2, max 4.5, margin 0.125), so the
        // crosshair picks it and the attack indicator shows.
        let spear = serde_json::json!({
            "min_reach": 2.0, "max_reach": 4.5,
            "min_creative_reach": 2.0, "max_creative_reach": 6.5,
            "hitbox_margin": 0.125,
        });
        let (eye, look) = (Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0));
        let boxes = mob_ahead(4.0);
        let reach = attack_reach(&spear, false);
        let pick = reach_pick(&boxes, eye, look, Vec3::ZERO, reach, |_, _| None);
        assert_eq!(pick, Some(4.0));
        assert!(attack_range_contains(&spear, 4.0, false));
        // Past the maximum plus the margin: no pick.
        let far = mob_ahead(4.7);
        assert_eq!(
            reach_pick(&far, eye, look, Vec3::ZERO, reach, |_, _| None),
            None
        );
        // A block nearer than the minimum reach takes the pick.
        let wall = |from: Vec3, _: Vec3| Some(from.add_vector(Vec3::new(0.0, 0.0, 1.0)));
        assert_eq!(reach_pick(&boxes, eye, look, Vec3::ZERO, reach, wall), None);
    }

    #[test]
    fn the_server_takes_attacks_within_vanilla_reach_only() {
        // handleAttack: AttackRange.defaultFor (0 to the entity interaction
        // range, no margin) plus 3, inclusive: 6 in survival, 8 in creative.
        let survival = (0.0, 3.0, 0.0);
        assert!(in_attack_range(6.0, survival, 3.0));
        assert!(!in_attack_range(6.01, survival, 3.0));
        assert!(!in_attack_range(50.0, survival, 3.0));
        let creative = (0.0, 5.0, 0.0);
        assert!(in_attack_range(8.0, creative, 3.0));
        assert!(!in_attack_range(8.01, creative, 3.0));
        // A spear's own range, with its margin, from the server's side too.
        let spear = serde_json::json!({"min_reach": 2.0, "max_reach": 4.5, "hitbox_margin": 0.125});
        let reach = attack_reach(&spear, false);
        assert!(in_attack_range(7.625, reach, 3.0));
        assert!(!in_attack_range(7.7, reach, 3.0));
    }

    #[test]
    fn spear_default_reach_and_bounds() {
        // #14092: A spear's default reach matches REACH (min 2.0, max 4.5, creative 6.5, margin 0.125).
        let r = &sunrise_sim::spear_sim::REACH;
        let spear = serde_json::json!({
            "min_reach": r.min, "max_reach": r.max,
            "min_creative_reach": r.min_creative, "max_creative_reach": r.max_creative,
            "hitbox_margin": r.hitbox_margin,
        });
        let survival = attack_reach(&spear, false);
        assert_eq!(survival, (2.0, 4.5, 0.125));
        let creative = attack_reach(&spear, true);
        assert_eq!(creative, (2.0, 6.5, 0.125));
        // Serverbound handleAttack allows reach plus 3.0 buffer:
        // Survival: 4.5 + 0.125 + 3.0 = 7.625
        assert!(in_attack_range(7.625, survival, 3.0));
        assert!(!in_attack_range(7.626, survival, 3.0));
        // Minimum reach boundary: 2.0 - 0.125 = 1.875
        assert!(attack_range_contains(&spear, 1.875, false));
        assert!(!attack_range_contains(&spear, 1.874, false));
        assert!(attack_range_contains(&spear, 4.625, false));
        assert!(!attack_range_contains(&spear, 4.626, false));
    }

    fn stack(item: &str, count: u32, damage: u32) -> Option<ItemStack> {
        Some(ItemStack {
            item: item.into(),
            count,
            damage,
            extra: None,
        })
    }

    #[test]
    fn a_swapped_in_stack_of_the_same_item_re_equips() {
        // #2866: F swaps 64 torches into the off hand that held 32. The off
        // hand's slot stays, but the server's slot update puts another stack
        // object there; a different count fails matchesIgnoringComponents,
        // so the hand plays the swap animation.
        let visible = (OFFHAND, stack("minecraft:torch", 32, 0));
        let held = (OFFHAND, stack("minecraft:torch", 64, 0));
        assert!(!shows_held_stack(&visible, &held, true));
        // The same stack shrunk in place (a torch placed): no animation.
        assert!(shows_held_stack(&visible, &held, false));
        // An equal stack replaced it: shown at once.
        assert!(shows_held_stack(&visible, &visible.clone(), true));
        // DataComponents.DAMAGE ignores the swap animation: a sword with
        // other damage swapped in shows at once.
        let sword = (OFFHAND, stack("minecraft:iron_sword", 1, 3));
        let worn = (OFFHAND, stack("minecraft:iron_sword", 1, 40));
        assert!(shows_held_stack(&sword, &worn, true));
        // Another component (an anvil's repair cost) differs.
        let mut named = worn.clone();
        named.1.as_mut().unwrap().data_mut().repair_cost = 1;
        assert!(!shows_held_stack(&sword, &named, true));
        // The main hand changing hotbar slot to an equal stack: instant; to
        // a different count: animation.
        let (a, b) = (
            (0, stack("minecraft:torch", 32, 0)),
            (1, stack("minecraft:torch", 32, 0)),
        );
        assert!(shows_held_stack(&a, &b, false));
        let c = (1, stack("minecraft:torch", 5, 0));
        assert!(!shows_held_stack(&a, &c, false));
        assert!(shows_held_stack(&(0, None), &(OFFHAND, None), true));
    }

    #[test]
    fn attack_range_uses_inclusive_float_bounds() {
        let range = serde_json::json!({
            "min_reach": 2.0, "max_reach": 4.5,
            "min_creative_reach": 2.0, "max_creative_reach": 6.5,
            "hitbox_margin": 0.125,
        });
        for (creative, min, max) in [(false, 1.875, 4.625), (true, 1.875, 6.625)] {
            for distance in [min, max] {
                assert!(attack_range_contains(&range, distance, creative));
            }
            for distance in [min - 0.0001, max + 0.0001] {
                assert!(!attack_range_contains(&range, distance, creative));
            }
        }
        let defaults = serde_json::json!({});
        for (creative, reach) in [(false, 3.0f32), (true, 5.0f32)] {
            let max = f64::from(reach + 0.3f32);
            assert!(attack_range_contains(&defaults, max, creative));
            assert!(!attack_range_contains(&defaults, max + 1e-8, creative));
        }
    }
}
