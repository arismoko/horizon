use super::level_sounds_common::LevelSound;
use crate::game::ServerGame;
use mod_api::SoundSource;
use sunrise_core::nbt::{Compound, Tag};
use sunrise_core::random::RandomSource;
use sunrise_gameplay_common::game::copper_golems::RECHECK_TICKS;
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;
use sunrise_sim::mobs::{
    Mob, Species,
    transport_items::{held, set_held},
};
use sunrise_world::chunk::BlockState;

impl ServerGame<'_> {
    /// A copper golem opened or closed a chest half: the counter changes, the
    /// first opener posts CONTAINER_OPEN and the last closer CONTAINER_CLOSE
    /// (`ContainerOpenersCounter.incrementOpeners` / `decrementOpeners`), and
    /// the clients see the block event (`openerCountChanged`).
    pub(in crate::game) fn golem_chest_open(&mut self, chest: [i32; 3], open: bool, mob: u32) {
        let key = (chest[0], chest[1], chest[2]);
        if self.common.mod_menu_at(key).is_some() {
            self.mod_container_opener(key, open, Some(sunrise_sim::mobs::Who::Mob(mob)), 3.0);
            return;
        }
        let entry = self
            .common
            .world
            .chest_openers
            .entry(key)
            .or_insert((0, 0.0));
        let before = entry.0;
        if open {
            entry.0 += 1;
            entry.1 = entry.1.max(3.0);
        } else {
            entry.0 -= 1;
            if entry.0 == 0 {
                entry.1 = 0.0;
            }
        }
        let count = entry.0;
        if count <= 0 {
            self.common.world.chest_openers.remove(&key);
        }
        let who = Some(sunrise_sim::mobs::Who::Mob(mob));
        if open && before == 0 {
            self.post_block_game_event("minecraft:container_open", chest, who, None);
        } else if !open && count == 0 {
            self.post_block_game_event("minecraft:container_close", chest, who, None);
        }
        self.block_entity_event(chest, 1, count.max(0));
        // The guests get the same `ClientboundBlockEventPacket` (the lid).
        self.lan_block_event(chest, 1, count.max(0));
    }
    /// A copper golem took from or filled a chest slot.
    pub(in crate::game) fn golem_chest_slot(
        &mut self,
        chest: [i32; 3],
        slot: usize,
        stack: Option<ItemStack>,
    ) {
        let key = (chest[0], chest[1], chest[2]);
        if let Some(e) = self.common.world.block_entities.get_mut(&key)
            && let Some(s) = e.slot_mut(slot)
        {
            *s = stack;
            // TransportItemsBetweenContainers calls Container.setChanged.
            self.container_changed(key);
        }
    }
    /// `CopperGolem.mobInteract`; true when handled.
    pub fn interact_copper_golem(&mut self, index: usize, item: &str) -> bool {
        if self.common.mobs[index].species() != Species::CopperGolem
            || !self.common.mobs[index].alive()
        {
            return false;
        }
        let at = {
            let m = &self.common.mobs[index];
            [m.position.x, m.position.y, m.position.z]
        };
        if item.is_empty() {
            // Empty hand: the golem throws what it carries back.
            let Some(h) = held(&self.common.mobs[index]).cloned() else {
                return false;
            };
            set_held(&mut self.common.mobs[index], None);
            // BehaviorUtils.throwItem: from 0.3 below the golem's eyes towards
            // the player's feet at 0.3 blocks a tick, default pickup delay.
            let p = self.common.player.as_ref().map(|p| p.position);
            let eye = self.common.mobs[index].eye_height();
            let from = [at[0], at[1] + eye - f64::from(0.3f32), at[2]];
            let v = p.map_or([0.0; 3], |p| {
                let d = [p.x - at[0], p.y - at[1], p.z - at[2]];
                let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                if len < 1.0e-5 {
                    return [0.0; 3];
                }
                let k = f64::from(0.3f32);
                [d[0] / len * k, d[1] / len * k, d[2] / len * k]
            });
            self.spawn_item(h, from, v, 10);
            return true;
        }
        let name = item;
        let is_axe = name.ends_with("_axe");
        let waxed = self.common.mobs[index].extra.vector[0] as i64 == -2;
        if self.held_shears(sunrise_gameplay_common::game::item_abilities::ability::SHEARS_HARVEST)
            && self.common.golem_shearable(index)
        {
            // CopperGolem.shear: the sound, the antenna slot emptied (an
            // unequip) and the flower dropped 1.5 up.
            // level.playSound(null, this, COPPER_GOLEM_SHEAR, PLAYERS, 1, 1).
            let shear = LevelSound::new("minecraft:entity.copper_golem.shear", at, 1.0, 1.0)
                .in_source(SoundSource::Players);
            self.play_bound_sound(shear, self.common.mobs[index].id, None);
            let antenna = self.common.mobs[index].saddle_item();
            self.common.mobs[index].set_saddle(None);
            let id = self.common.mobs[index].id;
            self.mob_equip_event(id, at, false);
            if let Some(a) = antenna {
                let p = self.common.mobs[index].position;
                self.spawn_at_location(p, 1.5, a);
            }
            // mobInteract: gameEvent(SHEAR, player) after the shear, before the
            // tool's damage.
            let ctx = self.who_context(Some(sunrise_sim::mobs::Who::Player), None);
            self.post_game_event("minecraft:shear", at, ctx);
            self.damage_held_tool();
            return true;
        }
        let block = {
            let p = self.common.mobs[index].position;
            [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32]
        };
        if name == "minecraft:honeycomb" && !waxed {
            self.common.mobs[index].extra.vector[0] = -2.0;
            self.server_level_event_at(3003, block, 0);
            self.server_play_sound("minecraft:item.honeycomb.wax_on", at, 1.0, 1.0);
            self.consume_held();
            return true;
        }
        if is_axe && waxed {
            self.common.mobs[index].extra.vector[0] = -1.0;
            // level.playSound(null, this, AXE_SCRAPE, getSoundSource(), 1, 1).
            let scrape = LevelSound::new("minecraft:item.axe.scrape", at, 1.0, 1.0)
                .in_source(SoundSource::Neutral);
            self.play_bound_sound(scrape, self.common.mobs[index].id, None);
            self.server_level_event_at(3004, block, 0);
            self.damage_held_tool();
            return true;
        }
        if is_axe && self.common.mobs[index].extra.variant > 0 {
            let m = &mut self.common.mobs[index];
            m.extra.vector[0] = -1.0;
            m.extra.variant = sunrise_sim::mobs::copper_golem::weather_previous(m.extra.variant);
            let id = m.id;
            self.common.persist.force_meta(id, 16);
            let scrape = LevelSound::new("minecraft:item.axe.scrape", at, 1.0, 1.0)
                .in_source(SoundSource::Neutral);
            self.play_bound_sound(scrape, id, None);
            self.server_level_event_at(3005, block, 0);
            self.damage_held_tool();
            return true;
        }
        false
    }
    /// `OfferFlowerGoal.stop` (`MobEvent::Gift`): the item on the target's
    /// empty antenna, as a guaranteed drop.
    pub fn receive_gift(&mut self, target: u32, item: &'static str) {
        if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == target)
            && m.saddle_item().is_none()
        {
            m.set_saddle(Some(ItemStack::new(item, 1)));
            // OfferFlowerGoal.stop: Mob.setGuaranteedDrop(antenna).
            sunrise_sim::mobs::finalize::set_drop_chance(m, sunrise_items::equipment::SADDLE, 2.0);
        }
    }
    /// `ContainerOpenersCounter.recheckOpeners` every five ticks.
    pub(in crate::game) fn recheck_golem_openers(&mut self) {
        if self.common.world.game_time % RECHECK_TICKS != 0 {
            return;
        }
        let open: Vec<(i32, i32, i32)> = self.common.world.chest_openers.keys().copied().collect();
        for key in open {
            let p = [key.0, key.1, key.2];
            let other = self.common.chest_other_half(key).map(|o| [o.0, o.1, o.2]);
            let range = self
                .common
                .world
                .chest_openers
                .get(&key)
                .map_or(0.0, |o| o.1)
                + 4.0;
            let area = sunrise_core::geometry::Aabb::new(
                sunrise_core::geometry::Vec3::new(
                    f64::from(p[0]) - range,
                    f64::from(p[1]) - range,
                    f64::from(p[2]) - range,
                ),
                sunrise_core::geometry::Vec3::new(
                    f64::from(p[0]) + 1.0 + range,
                    f64::from(p[1]) + 1.0 + range,
                    f64::from(p[2]) + 1.0 + range,
                ),
            );
            let count = self
                .common
                .mobs
                .iter()
                .filter(|m| {
                    m.species() == Species::CopperGolem
                        && !m.removed
                        && m.extra
                            .opened_chest
                            .is_some_and(|o| o == p || Some(o) == other)
                        && m.bounds().intersects(area)
                })
                .count() as i32;
            let before = self.common.world.chest_openers.get(&key).map_or(0, |o| o.0);
            if count == before {
                continue;
            }
            if count == 0 {
                self.common.world.chest_openers.remove(&key);
                self.golem_chest_sound(p, false);
                self.post_block_game_event("minecraft:container_close", p, None, None);
            } else if let Some(o) = self.common.world.chest_openers.get_mut(&key) {
                o.0 = count;
            }
            self.block_entity_event(p, 1, count);
            self.lan_block_event(p, 1, count);
        }
    }
    /// `ChestBlockEntity.playSound` for a chest the recheck closed.
    pub(in crate::game::server_side) fn golem_chest_sound(&mut self, p: [i32; 3], open: bool) {
        let name = self
            .common
            .res
            .facts()
            .get(self.common.world.block(p[0], p[1], p[2]))
            .name
            .clone();
        let base = name
            .trim_start_matches("minecraft:")
            .trim_start_matches("waxed_");
        let kind = if !base.ends_with("copper_chest") {
            "chest"
        } else if base.starts_with("weathered") {
            "copper_chest_weathered"
        } else if base.starts_with("oxidized") {
            "copper_chest_oxidized"
        } else {
            "copper_chest"
        };
        let side = if open { "open" } else { "close" };
        let at = [
            f64::from(p[0]) + 0.5,
            f64::from(p[1]) + 0.5,
            f64::from(p[2]) + 0.5,
        ];
        let pitch = self.common.world.random.next_float() * 0.1 + 0.9;
        self.server_play_sound(&format!("minecraft:block.{kind}.{side}"), at, 0.5, pitch);
    }
    /// `CopperGolem.turnToStatue`, which its weathering asked for.
    pub fn golem_become_statue(&mut self, id: u32) {
        let Some(i) = self
            .common
            .mobs
            .iter()
            .position(|m| m.id == id && !m.removed)
        else {
            return;
        };
        let statue_at = self.with_entity_held_out(id, |game| {
            let mut m = game.mobs[i].clone();
            let became_statue = game.golem_turn_to_statue(&mut m);
            let position = m.position.components();
            if let Some(i) = game.mobs.iter().position(|x| x.id == id && !x.removed) {
                game.mobs.update(i, |owner| {
                    sunrise_sim::join_seq::store_back(owner, m);
                });
            }
            became_statue.then_some(position)
        });
        if let Some(at) = statue_at {
            self.remove_mob_now(id, false);
            self.server_play_sound("minecraft:entity.copper_golem_become_statue", at, 1.0, 1.0);
            if self
                .common
                .mob_object(id)
                .is_some_and(|mob| mob.extra.leash.is_some())
            {
                let drop = self
                    .common
                    .ui
                    .rules
                    .bool_at(sunrise_game_data::gamerules::vanilla::ENTITY_DROPS);
                self.break_leash(id, drop, false);
            }
        }
    }
    pub(in crate::game::server_side) fn golem_turn_to_statue(&mut self, m: &mut Mob) -> bool {
        let pos = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ];
        let pose = ["standing", "sitting", "running", "star"]
            [m.random.next_int_bound(4).unwrap_or(0) as usize];
        let facing = ["south", "west", "north", "east"]
            [((f64::from(m.yaw) / 90.0 + 0.5).floor() as i32 & 3) as usize];
        let state = BlockState {
            name: "minecraft:oxidized_copper_golem_statue".into(),
            properties: [
                ("facing".to_string(), facing.to_string()),
                ("copper_golem_pose".to_string(), pose.to_string()),
                ("waterlogged".to_string(), "false".to_string()),
            ]
            .into(),
        };
        let registry = self.common.pack.assets.registry;
        let Some(id) = registry.id(&state) else {
            return false;
        };
        if self.level_set(pos, id).is_err() {
            return false;
        }
        let mut components = Compound::new();
        if let Some(name) = self.golem_name_tag(m) {
            components.insert("minecraft:custom_name".into(), name);
        }
        self.common.world.block_entities.insert(
            (pos[0], pos[1], pos[2]),
            BlockEntity::CopperGolemStatue(components),
        );
        self.golem_drop_equipment(m);
        m.extra.opened_chest = None;
        true
    }
    pub(in crate::game::server_side) fn golem_name_tag(&self, m: &Mob) -> Option<Tag> {
        let plain = m.custom_name.as_ref()?;
        let saved = self
            .common
            .persist
            .raw
            .get(&m.id)
            .and_then(|raw| raw.get("CustomName"))
            .filter(|name| match name {
                Tag::String(s) => s.to_string_lossy() == *plain,
                Tag::Compound(_) => matches!(
                    name.get("text"),
                    Some(Tag::String(s)) if s.to_string_lossy() == *plain
                ),
                _ => false,
            });
        Some(
            saved
                .cloned()
                .unwrap_or_else(|| Tag::String(plain.as_str().into())),
        )
    }
    pub(in crate::game::server_side) fn name_golem(&mut self, id: u32, name: Tag) {
        let plain = match &name {
            Tag::String(s) => Some(s.to_string_lossy()),
            Tag::Compound(_) => match name.get("text") {
                Some(Tag::String(s)) => Some(s.to_string_lossy()),
                _ => None,
            },
            _ => None,
        };
        if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
            m.custom_name = plain;
            m.custom_name_text = Some(sunrise_sim::entity_nbt::name_text(&name));
        }
        if let Tag::Compound(raw) = self
            .common
            .persist
            .raw
            .entry(id)
            .or_insert_with(|| Tag::Compound(Default::default()))
        {
            raw.insert("CustomName".into(), name);
        }
    }
    pub(in crate::game) fn golem_drop_equipment(&mut self, m: &mut Mob) {
        let at = m.position;
        for stack in sunrise_gameplay_common::game::golem_preserved_equipment(m) {
            self.spawn_at_location(at, 0.0, stack);
        }
    }
    pub(in crate::game) fn use_golem_statue(
        &mut self,
        pos: [i32; 3],
        held: Option<&ItemStack>,
    ) -> bool {
        if !held.is_some_and(|s| s.item.ends_with("_axe")) {
            return false;
        }
        let registry = self.common.pack.assets.registry;
        let Some(state) = registry.state(self.common.world.block(pos[0], pos[1], pos[2])) else {
            return false;
        };
        if state.name != "minecraft:copper_golem_statue" {
            return false;
        }
        let yaw = match state.properties.get("facing").map(String::as_str) {
            Some("west") => 90.0,
            Some("north") => 180.0,
            Some("east") => 270.0,
            _ => 0.0,
        };
        let waterlogged = state
            .properties
            .get("waterlogged")
            .is_some_and(|w| w == "true");
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]),
            f64::from(pos[2]) + 0.5,
        ];
        let name = match self
            .common
            .world
            .block_entities
            .get(&(pos[0], pos[1], pos[2]))
        {
            Some(BlockEntity::CopperGolemStatue(components)) => {
                components.get(&"minecraft:custom_name".into()).cloned()
            }
            _ => None,
        };
        let Some(id) = self.spawn_mob_raw("minecraft:copper_golem", at, true) else {
            return false;
        };
        if let Some(name) = name {
            self.name_golem(id, name);
        }
        if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
            m.yaw = yaw;
            m.previous_yaw = yaw;
            m.body_yaw = yaw;
            m.previous_body_yaw = yaw;
            m.head_yaw = yaw;
            m.previous_head_yaw = yaw;
        }
        self.damage_held(1);
        self.server_play_sound("minecraft:entity.copper_golem.spawn", at, 1.0, 1.0);
        let replacement = registry.default_state(if waterlogged {
            "minecraft:water"
        } else {
            "minecraft:air"
        });
        if let Some(replacement) = replacement {
            let _ = self.level_set(pos, replacement);
        }
        self.common
            .world
            .block_entities
            .remove(&(pos[0], pos[1], pos[2]));
        true
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;
