//! Living entities for native server mods, through a `LivingRef`
//! (docs/native-mods.md, "Entity and level handles", #7387): the equipment
//! of mobs, players and armor stands. Reads return `None`, writes false, for
//! a stale handle or on a client.
use super::Game;
use super::commands::{Target, items::Owner};
use horizon_client::first_person::Hand;
use horizon_client::items::ItemStack;
use horizon_client::mod_api::{EntityRef, EquipmentSlot, LivingRef};

/// A player's armour slots (`armor`, head first) by equipment slot.
fn armor_index(slot: EquipmentSlot) -> Option<usize> {
    match slot {
        EquipmentSlot::Head => Some(0),
        EquipmentSlot::Chest => Some(1),
        EquipmentSlot::Legs => Some(2),
        EquipmentSlot::Feet => Some(3),
        _ => None,
    }
}

/// A player's stack in `slot`, from its inventory, armour and off hand.
fn player_slot(
    inventory: &[Option<ItemStack>],
    selected: usize,
    armor: &[Option<ItemStack>; 4],
    offhand: &Option<ItemStack>,
    equipment: &horizon_items::equipment::Equipment,
    slot: EquipmentSlot,
) -> Option<ItemStack> {
    match slot {
        EquipmentSlot::MainHand => inventory.get(selected).cloned().flatten(),
        EquipmentSlot::OffHand => offhand.clone(),
        EquipmentSlot::Body | EquipmentSlot::Saddle => equipment.get(slot as usize).cloned(),
        _ => armor[armor_index(slot)?].clone(),
    }
}

/// The `/item` slot name of an equipment slot.
fn slot_name(slot: EquipmentSlot) -> &'static str {
    match slot {
        EquipmentSlot::MainHand => "weapon.mainhand",
        EquipmentSlot::OffHand => "weapon.offhand",
        EquipmentSlot::Head => "armor.head",
        EquipmentSlot::Chest => "armor.chest",
        EquipmentSlot::Legs => "armor.legs",
        EquipmentSlot::Feet => "armor.feet",
        EquipmentSlot::Body => "armor.body",
        EquipmentSlot::Saddle => "saddle",
    }
}

/// `LivingEntity.isAlive`'s health test: above 0, so a NaN health loaded from
/// a save is not alive either.
fn alive_at(health: f32) -> bool {
    health > 0.0
}

fn equipment_stack(stack: Option<ItemStack>) -> Option<ItemStack> {
    stack.filter(|s| s.count > 0 && &*s.item != "minecraft:air")
}

impl Game<'_> {
    /// What a living handle names now: a player, a mob, an armor stand or the dragon.
    pub(super) fn native_living_target(&self, living: LivingRef) -> Option<Target> {
        match self.native_entity_target(living.entity())? {
            t @ (Target::Player
            | Target::Guest(_)
            | Target::Mob(_)
            | Target::Stand(_)
            | Target::Dragon(_)) => Some(t),
            _ => None,
        }
    }
    /// The entity as a living entity (vanilla's `LivingEntity`: a mob, a
    /// player, an armor stand or the dragon), or None for another kind or a
    /// stale handle.
    pub(super) fn native_living(&self, entity: EntityRef) -> Option<LivingRef> {
        let living = LivingRef::new(entity);
        self.native_living_target(living).map(|_| living)
    }
    /// The effective value of an attribute on a living entity. None for an
    /// unknown or absent attribute, an unavailable entity, or on a client.
    /// Mobs, armor stands and the dragon return None until they have live
    /// attribute maps.
    pub(super) fn native_living_attribute_value(
        &mut self,
        living: LivingRef,
        id: &str,
    ) -> Option<f64> {
        self.native_living_attribute(
            living,
            id,
            horizon_world::attribute_map::ValueKind::Computed,
        )
    }

    /// The base value of an attribute, before modifiers. Mobs, armor stands
    /// and the dragon return None until they have live attribute maps.
    pub(super) fn native_living_attribute_base(
        &mut self,
        living: LivingRef,
        id: &str,
    ) -> Option<f64> {
        self.native_living_attribute(living, id, horizon_world::attribute_map::ValueKind::Base)
    }

    fn native_living_attribute(
        &mut self,
        living: LivingRef,
        name: &str,
        kind: horizon_world::attribute_map::ValueKind,
    ) -> Option<f64> {
        let player = match self.native_living_target(living)? {
            Target::Player => self.player.as_mut()?,
            Target::Guest(conn) => self
                .lan
                .as_mut()?
                .hub
                .guest_mut(conn)?
                .motion
                .as_deref_mut()?,
            _ => return None,
        };
        let key = player.attributes.key(name)?;
        player.attributes.value(Some(key), kind).ok()
    }

    /// The entity's health (`LivingEntity.getHealth`; Paper's
    /// `Damageable#getHealth`). During a player's turn only that player
    /// resolves: the hub is out for their action, and other players'
    /// handles read `None` until it is back.
    pub(super) fn native_living_health(&self, living: LivingRef) -> Option<f32> {
        match self.native_living_target(living)? {
            Target::Player => Some(self.player.as_ref()?.health.health()),
            Target::Guest(conn) => Some(self.command_guest(conn)?.health),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)
                .map(|m| m.health),
            // An armor stand carries the fixed 20 the game gives it: lava
            // does not hurt it and splash water reads the very field.
            Target::Stand(id) => self
                .armor_stands
                .iter()
                .find(|a| a.id == id)
                .map(|a| a.health),
            Target::Dragon(id) => self
                .dragon
                .as_ref()
                .filter(|d| d.id == id)
                .map(|d| d.health),
            _ => None,
        }
    }
    /// The entity's maximum health (`LivingEntity.getMaxHealth`; Paper's
    /// `Damageable#getMaxHealth`). Players and guests read their attribute
    /// map; a mob, an armor stand and the dragon their own fields (the
    /// armor stand the fixed 20 the game seeds it with, the dragon its
    /// 200) until live attribute maps cover them.
    pub(super) fn native_living_max_health(&mut self, living: LivingRef) -> Option<f32> {
        match self.native_living_target(living)? {
            Target::Player | Target::Guest(_) => self
                .native_living_attribute(
                    living,
                    "minecraft:max_health",
                    horizon_world::attribute_map::ValueKind::Computed,
                )
                .map(|max| max as f32),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)
                .map(|m| m.attrs.max_health),
            Target::Stand(_) => Some(20.0),
            Target::Dragon(_) => Some(super::dragon::MAX_HEALTH),
            _ => None,
        }
    }
    /// Whether the entity is alive (`LivingEntity.isAlive`; Paper's
    /// `Entity#isDead` is its inverse). A dying mob, mid death
    /// animation, is not.
    pub(super) fn native_living_alive(&self, living: LivingRef) -> Option<bool> {
        self.native_living_health(living).map(alive_at)
    }

    /// The stack in `slot` (`LivingEntity.getItemBySlot`). Players, guests,
    /// mobs and armor stands store main hand, off hand, head, chest, legs and
    /// feet. Players, guests and mobs also store body and saddle. Dragon
    /// equipment and armor-stand body and saddle storage are not yet
    /// modeled. None means empty, unsupported or stale.
    pub(super) fn native_living_equipment(
        &self,
        living: LivingRef,
        slot: EquipmentSlot,
    ) -> Option<ItemStack> {
        match self.native_living_target(living)? {
            Target::Player => player_slot(
                &self.inventory,
                self.selected,
                &self.armor,
                &self.offhand,
                &self.equipment,
                slot,
            ),
            Target::Guest(conn) => {
                let g = self.command_guest(conn)?;
                player_slot(
                    &g.inventory,
                    g.selected,
                    &g.armor,
                    &g.offhand,
                    &g.equipment,
                    slot,
                )
            }
            Target::Mob(id) => {
                self.mobs.iter().find(|m| m.id == id)?.equipment_stacks()[slot as usize].clone()
            }
            Target::Stand(id) => self
                .armor_stands
                .iter()
                .find(|a| a.id == id)?
                .equipment
                .get(slot as usize)?
                .clone(),
            _ => None,
        }
    }
    /// The stack in hand `hand` (`LivingEntity.getItemInHand`).
    pub(super) fn native_living_hand_item(
        &self,
        living: LivingRef,
        hand: Hand,
    ) -> Option<ItemStack> {
        let slot = match hand {
            Hand::Main => EquipmentSlot::MainHand,
            Hand::Off => EquipmentSlot::OffHand,
        };
        self.native_living_equipment(living, slot)
    }
    /// Puts `stack` in `slot` (`LivingEntity.setItemSlot`); None, air or a
    /// zero count empties it. Players, guests, mobs and armor stands store
    /// main hand, off hand, head, chest, legs and feet. Players, guests and
    /// mobs also store body and saddle. Dragon equipment and armor-stand body
    /// and saddle storage are not yet modeled. False means unsupported or stale.
    /// This is a direct write: it takes effect and is final immediately, even
    /// inside `host.transaction`. Any equip/unequip event and equip sound also
    /// happen immediately.
    pub(super) fn native_living_set_equipment(
        &mut self,
        living: LivingRef,
        slot: EquipmentSlot,
        stack: Option<ItemStack>,
    ) -> bool {
        let Some(t) = self.native_living_target(living) else {
            return false;
        };
        let stack = equipment_stack(stack);
        if let Target::Mob(id) = t {
            let equippable = stack.as_ref().and_then(|s| {
                let name = horizon_client::item_sync::equippable_slot(s, self.items())?;
                let equip_slot = super::dispenser::Slot::from_name(&name)?;
                let patched = s.data().and_then(|d| d.other("minecraft:equippable"));
                let sound = if let Some(component) = patched {
                    match component.get("equip_sound") {
                        Some(horizon_world::nbt::Tag::String(sound)) => {
                            Some(sound.to_string_lossy())
                        }
                        _ => None,
                    }
                } else {
                    self.items()
                        .get(&s.item)
                        .and_then(|i| i.equip_sound.clone())
                }
                .unwrap_or_else(|| "minecraft:item.armor.equip_generic".into());
                Some((equip_slot, sound))
            });
            let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id) else {
                return false;
            };
            let old = if (slot as usize) <= 5 {
                horizon_client::mobs::finalize::item_stack(mob, slot as usize)
            } else {
                mob.equipment_stacks()[slot as usize].clone()
            };
            let changed = match (&old, &stack) {
                (None, None) => false,
                (Some(a), Some(b)) => !a.same_item_same_components(b),
                _ => true,
            };
            if !mob.set_equipment_stack(slot as usize, stack) {
                return false;
            }
            if changed && !mob.first_tick {
                let at = mob.position;
                let sound = equippable.as_ref().and_then(|(equip_slot, sound)| {
                    (!mob.silent && *equip_slot as usize == slot as usize)
                        .then(|| {
                            super::dispenser::equip_sound(mob, *equip_slot, Some(sound.clone()))
                        })
                        .flatten()
                });
                if let Some(sound) = sound {
                    self.audio.play(&sound, Some([at.x, at.y, at.z]), 1.0, 1.0);
                }
                let ctx = self.who_context(Some(horizon_client::mobs::Who::Mob(id)), None);
                self.post_game_event(
                    if equippable.is_some() {
                        "minecraft:equip"
                    } else {
                        "minecraft:unequip"
                    },
                    [at.x, at.y, at.z],
                    ctx,
                );
            }
            return true;
        }
        match self
            .slots_named(&Owner::Entity(t), slot_name(slot))
            .as_slice()
        {
            [s] => self.slot_set(s, stack),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn living_reads_answer_the_vanilla_defaults_and_a_gone_entity_reads_none() {
        use horizon_client::entities::ArmorStand;
        use horizon_client::mobs::Mob;
        use horizon_client::motion::PlayerMotion;
        use horizon_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            game.armor_stands.push(ArmorStand::new(13, Vec3::ZERO, 0.0));
            let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 14;
            game.dragon = Some(dragon);
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.lan_acting_uuid = Some(123);
            game.lan_acting_entity = Some(20);
            // The vanilla attribute defaults: a player and a zombie 20, an
            // armor stand the 20 the game seeds it with, the dragon its 200.
            for (id, health, max, alive) in [
                (20u32, 20.0, 20.0, true),
                (12, 20.0, 20.0, true),
                (13, 20.0, 20.0, true),
                (14, 200.0, 200.0, true),
            ] {
                let living = game.native_living(game.native_entity(id).unwrap()).unwrap();
                assert_eq!(living.health(game), Some(health), "health of {id}");
                assert_eq!(living.max_health(game), Some(max), "max of {id}");
                assert_eq!(living.alive(game), Some(alive), "alive of {id}");
            }
            // A dead dragon: isAlive is health above zero.
            let dragon_handle = game.native_living(game.native_entity(14).unwrap()).unwrap();
            game.dragon.as_mut().unwrap().health = 0.0;
            assert_eq!(dragon_handle.health(game), Some(0.0));
            assert_eq!(dragon_handle.alive(game), Some(false));
            // The entities gone: every read answers None.
            game.player = None;
            game.lan_acting_uuid = None;
            game.lan_acting_entity = None;
            game.mobs[0].removed = true;
            game.armor_stands.clear();
            game.dragon = None;
            for id in [20u32, 12, 13, 14] {
                let handle = game.native_entity(id).and_then(|er| game.native_living(er));
                assert!(handle.is_none(), "handle of {id} after it is gone");
            }
            assert_eq!(dragon_handle.health(game), None);
            assert_eq!(dragon_handle.max_health(game), None);
            assert_eq!(dragon_handle.alive(game), None);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn mob_equipment_writes_emit_vanilla_equip_events() {
        use horizon_client::mobs::Mob;
        use horizon_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, kind, Vec3::ZERO, 0.0));
            let living = game.native_living(game.native_entity(12).unwrap()).unwrap();
            game.audio.capture = Some(Vec::new());
            super::super::dispense_fixture::start_recording();
            let helmet = ItemStack::new("minecraft:iron_helmet", 1);
            assert!(game.native_living_set_equipment(
                living,
                EquipmentSlot::Head,
                Some(helmet.clone())
            ));
            assert!(game.native_living_set_equipment(living, EquipmentSlot::Head, None));
            assert!(game.audio.capture.as_ref().unwrap().is_empty());
            game.mobs[0].first_tick = false;
            assert!(game.native_living_set_equipment(
                living,
                EquipmentSlot::Head,
                Some(helmet.clone())
            ));
            assert_eq!(game.audio.capture.as_ref().unwrap().len(), 1);
            assert!(game.native_living_set_equipment(living, EquipmentSlot::Head, Some(helmet)));
            assert_eq!(game.audio.capture.as_ref().unwrap().len(), 1);
            game.mobs[0].silent = true;
            assert!(game.native_living_set_equipment(
                living,
                EquipmentSlot::Head,
                Some(ItemStack::new("minecraft:diamond_helmet", 1))
            ));
            assert!(game.native_living_set_equipment(living, EquipmentSlot::Head, None));
            game.mobs[0].silent = false;
            // Equippable, but its head slot does not match chest: event, no sound.
            assert!(game.native_living_set_equipment(
                living,
                EquipmentSlot::Chest,
                Some(ItemStack::new("minecraft:iron_helmet", 1))
            ));
            assert_eq!(game.audio.capture.as_ref().unwrap().len(), 1);
            let events = super::super::dispense_fixture::stop_recording();
            assert_eq!(
                events
                    .iter()
                    .filter(|e| e.ends_with("minecraft:equip"))
                    .count(),
                3
            );
            assert_eq!(
                events
                    .iter()
                    .filter(|e| e.ends_with("minecraft:unequip"))
                    .count(),
                1
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn mob_equipment_writes_keep_full_stacks() {
        use horizon_client::mobs::Mob;
        use horizon_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            for (id, kind) in [
                (12, "minecraft:zombie"),
                (13, "minecraft:horse"),
                (14, "minecraft:pillager"),
            ] {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
            }
            let zombie = game.native_living(game.native_entity(12).unwrap()).unwrap();
            let horse = game.native_living(game.native_entity(13).unwrap()).unwrap();
            let captain = game.native_living(game.native_entity(14).unwrap()).unwrap();
            let mut helmet = ItemStack::new("minecraft:iron_helmet", 2);
            helmet.damage = 7;
            helmet.data_mut().enchantments = vec![("minecraft:protection".into(), 3)];
            assert!(zombie.set_equipment(game, EquipmentSlot::Head, Some(helmet.clone())));
            assert_eq!(
                zombie.equipment(game, EquipmentSlot::Head),
                Some(helmet.clone())
            );
            assert_eq!(
                game.mobs[0]
                    .extra
                    .slot_stack(horizon_items::equipment::HEAD),
                Some(&helmet)
            );
            let mut shield = ItemStack::new("minecraft:shield", 2);
            shield.damage = 11;
            assert!(game.native_living_set_equipment(
                zombie,
                EquipmentSlot::OffHand,
                Some(shield.clone())
            ));
            assert_eq!(
                game.native_living_equipment(zombie, EquipmentSlot::OffHand),
                Some(shield)
            );
            assert!(game.native_living_set_equipment(
                horse,
                EquipmentSlot::Head,
                Some(helmet.clone())
            ));
            let mut body = ItemStack::new("minecraft:diamond_horse_armor", 3);
            body.damage = 9;
            body.data_mut().custom_name = Some(horizon_world::nbt::Tag::String("armor".into()));
            assert!(game.native_living_set_equipment(
                horse,
                EquipmentSlot::Body,
                Some(body.clone())
            ));
            assert_eq!(
                game.native_living_equipment(horse, EquipmentSlot::Head),
                Some(helmet.clone())
            );
            assert_eq!(
                game.native_living_equipment(horse, EquipmentSlot::Body),
                Some(body)
            );
            for slot in [
                EquipmentSlot::MainHand,
                EquipmentSlot::Chest,
                EquipmentSlot::Legs,
                EquipmentSlot::Feet,
            ] {
                let stack = ItemStack::new("minecraft:stone", 4);
                assert!(game.native_living_set_equipment(zombie, slot, Some(stack.clone())));
                assert_eq!(game.native_living_equipment(zombie, slot), Some(stack));
                assert!(game.native_living_set_equipment(
                    zombie,
                    slot,
                    Some(ItemStack::new("minecraft:stone", 0))
                ));
                assert_eq!(game.native_living_equipment(zombie, slot), None);
            }
            let mut saddle = ItemStack::new("minecraft:saddle", 3);
            saddle.data_mut().custom_name = Some(horizon_world::nbt::Tag::String("saddle".into()));
            assert!(game.native_living_set_equipment(
                horse,
                EquipmentSlot::Saddle,
                Some(saddle.clone())
            ));
            assert_eq!(
                game.native_living_equipment(horse, EquipmentSlot::Saddle),
                Some(saddle)
            );
            horizon_client::mobs::raider::set_patrol_leader(&mut game.mobs[2], true);
            horizon_client::mobs::raider::wear_banner(&mut game.mobs[2]);
            assert!(horizon_client::mobs::raider::is_captain(&game.mobs[2]));
            assert!(game.native_living_set_equipment(
                captain,
                EquipmentSlot::Head,
                Some(helmet.clone())
            ));
            assert_eq!(
                game.native_living_equipment(captain, EquipmentSlot::Head),
                Some(helmet)
            );
            assert!(!horizon_client::mobs::raider::is_captain(&game.mobs[2]));
            assert!(game.native_living_set_equipment(
                horse,
                EquipmentSlot::Body,
                Some(ItemStack::new("minecraft:air", 1))
            ));
            assert_eq!(
                game.native_living_equipment(horse, EquipmentSlot::Body),
                None
            );
            let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 15;
            game.dragon = Some(dragon);
            let dragon = game.native_living(game.native_entity(15).unwrap()).unwrap();
            assert!(!game.native_living_set_equipment(
                dragon,
                EquipmentSlot::Saddle,
                Some(ItemStack::new("minecraft:saddle", 1))
            ));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_host_players_body_and_saddle_are_slots_and_saved_in_equipment() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let owner = Owner::Entity(Target::Player);
            let mut body = ItemStack::new("minecraft:diamond_horse_armor", 1);
            body.damage = 4;
            body.data_mut().custom_name = Some(horizon_world::nbt::Tag::String("armor".into()));
            let saddle = ItemStack::new("minecraft:saddle", 1);
            game.player = Some(
                horizon_client::motion::PlayerMotion::new(
                    game.motion,
                    horizon_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            game.audio.capture = Some(Vec::new());
            for (name, stack) in [("armor.body", &body), ("saddle", &saddle)] {
                let slots = game.slots_named(&owner, name);
                assert_eq!(slots.len(), 1, "{name} is one slot");
                assert!(game.slot_set(&slots[0], Some(stack.clone())));
            }
            assert_eq!(
                (game.equipment.get(6), game.equipment.get(7)),
                (Some(&body), Some(&saddle))
            );
            // onEquipItem: each item in its own slot plays its equip sound.
            assert_eq!(game.audio.capture.as_ref().unwrap().len(), 2);
            game.sync_level();
            let tag = game.level.player_tag();
            let equipment = tag.get("equipment").unwrap();
            assert!(equipment.get("body").is_some() && equipment.get("saddle").is_some());
            let mut read = horizon_client::save::Level::default();
            read.read_player(&tag);
            // Loading the file back into a Game restores both, components too.
            game.equipment = Default::default();
            let level = read;
            game.load_saved_inventory(&level);
            assert_eq!(
                (game.equipment.get(6), game.equipment.get(7)),
                (Some(&body), Some(&saddle))
            );
        });
    }

    #[test]
    fn alive_is_health_above_zero_as_in_vanilla() {
        assert!(alive_at(0.5));
        assert!(!alive_at(0.0));
        assert!(!alive_at(-1.0));
        assert!(!alive_at(f32::NAN));
    }

    #[test]
    fn air_and_zero_count_are_empty_on_write() {
        assert!(equipment_stack(Some(ItemStack::new("minecraft:air", 1))).is_none());
        assert!(equipment_stack(Some(ItemStack::new("minecraft:stone", 0))).is_none());
        assert!(equipment_stack(Some(ItemStack::new("minecraft:stone", 1))).is_some());
    }

    #[test]
    fn a_players_slots_come_from_its_inventory_armour_and_off_hand() {
        let stack = |id: &str| Some(ItemStack::new(id, 1));
        let inventory = vec![stack("minecraft:stone"), stack("minecraft:dirt")];
        let armor = [
            stack("minecraft:iron_helmet"),
            None,
            None,
            stack("minecraft:iron_boots"),
        ];
        let offhand = stack("minecraft:shield");
        let mut equipment = horizon_items::equipment::Equipment::default();
        equipment.set(horizon_items::equipment::SADDLE, stack("minecraft:saddle"));
        let at =
            |slot| player_slot(&inventory, 1, &armor, &offhand, &equipment, slot).map(|s| s.item);
        assert_eq!(
            at(EquipmentSlot::MainHand).as_deref(),
            Some("minecraft:dirt")
        );
        assert_eq!(
            at(EquipmentSlot::OffHand).as_deref(),
            Some("minecraft:shield")
        );
        assert_eq!(
            at(EquipmentSlot::Head).as_deref(),
            Some("minecraft:iron_helmet")
        );
        assert_eq!(
            at(EquipmentSlot::Feet).as_deref(),
            Some("minecraft:iron_boots")
        );
        assert_eq!(at(EquipmentSlot::Chest), None);
        assert_eq!(at(EquipmentSlot::Body), None);
        assert_eq!(
            at(EquipmentSlot::Saddle).as_deref(),
            Some("minecraft:saddle")
        );
    }

    #[test]
    fn every_equipment_slot_has_its_own_item_command_name() {
        let names = EquipmentSlot::ALL.map(slot_name);
        for (i, n) in names.iter().enumerate() {
            assert!(!names[..i].contains(n), "{n} named twice");
        }
    }
}
