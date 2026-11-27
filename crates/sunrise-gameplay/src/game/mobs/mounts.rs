//! Riding horses, donkeys and mules: feeding (`handleEating`), saddles,
//! armour and chests, mounting, the rider's control and jump bar
//! (`LocalPlayer.aiStep`'s riding jump), positioning, dismounting, and the
//! mount's health and jump bar on the HUD.
use crate::game::ServerGame;
use sunrise_core::{
    geometry::{Aabb, Vec3, VoxelShape},
    random::RandomSource,
};
use sunrise_gameplay_common::game::{item_abilities::ability, mounts_common::steering_item};
use sunrise_world::living_air::escape_vector;
use {
    sunrise_items::blocks::Fluid, sunrise_items::items::EntityTypeTags,
    sunrise_items::items::ItemStack, sunrise_sim::first_person_sim::Hand, sunrise_sim::mobs::Mob,
    sunrise_sim::mobs::MobEvent, sunrise_sim::mobs::Species, sunrise_sim::mobs::Who,
    sunrise_sim::mobs::equine, sunrise_sim::motion::MotionWorld, sunrise_sim::motion::PlayerMotion,
    sunrise_sim::pose::Dimensions, sunrise_sim::pose::Pose, sunrise_sim::sprint::Keys,
};

/// `AbstractHorse.isFood` and `handleEating`: what a mount eats, and what that
/// mouthful does — `(heal, age up seconds, temper, falls in love)`. The temper
/// increment is kept separate from its signed saved counter.
///
/// `AbstractHorse.isFood` is the `#minecraft:horse_food` tag, `ZombieHorse`
/// overrides it with `#minecraft:zombie_horse_food` (a red mushroom and
/// nothing else), and the horse food tag carries no red mushrooms; a skeleton
/// horse inherits the horse food tag but only a tamed one gets that far
/// (`SkeletonHorse.mobInteract` passes for an untamed one).
pub(in crate::game::server_side) fn eating(
    species: Species,
    item: &str,
) -> Option<(f32, i32, u8, bool)> {
    let camel = species == Species::Camel;
    // CamelHusk.isFood: rabbit's feet (and it never falls in love).
    if species == Species::CamelHusk && item != "minecraft:rabbit_foot" {
        return None;
    }
    if species != Species::CamelHusk && camel != (item == "minecraft:cactus") {
        return None;
    }
    let food = match species {
        Species::ZombieHorse => item == "minecraft:red_mushroom",
        Species::Horse | Species::Donkey | Species::Mule | Species::SkeletonHorse => {
            item != "minecraft:red_mushroom"
        }
        // Camels and llamas answer their own tags above or in the table.
        _ => true,
    };
    if !food {
        return None;
    }
    Some(match item {
        "minecraft:rabbit_foot" if species == Species::CamelHusk => (2.0, 10, 0, false),
        // Camel.handleEating.
        "minecraft:cactus" => (2.0, 10, 0, true),
        "minecraft:wheat" => (2.0, 20, 3, false),
        "minecraft:sugar" => (1.0, 30, 3, false),
        "minecraft:hay_block" => (20.0, 180, 0, false),
        "minecraft:apple" => (3.0, 60, 3, false),
        "minecraft:red_mushroom" => (3.0, 0, 3, false),
        "minecraft:carrot" => (3.0, 60, 3, false),
        "minecraft:golden_carrot" => (4.0, 60, 5, true),
        "minecraft:golden_apple" | "minecraft:enchanted_golden_apple" => (10.0, 240, 10, true),
        _ => return None,
    })
}
/// `EntityTypeTags.DISMOUNTS_UNDERWATER` (`Entity.dismountsUnderwater`): the mounts that throw their
/// riders off when the rider's eyes are in water.
pub fn dismounts_underwater(tags: &EntityTypeTags, mount: &Mob) -> bool {
    tags.has_bit(
        mount.kind.flags.entity_tag_slot,
        EntityTypeTags::DISMOUNTS_UNDERWATER,
    )
}

impl ServerGame<'_> {
    /// `FoodOnAStickItem.use` while riding: boost the mount, wear the stick.
    pub fn use_steering_stick(&mut self, item: &str) -> bool {
        let (species, wear) = match item {
            "minecraft:carrot_on_a_stick" => (Species::Pig, 7),
            "minecraft:warped_fungus_on_a_stick" => (Species::Strider, 1),
            _ => return false,
        };
        let Some(i) = self.common.mount_index() else {
            return false;
        };
        if self.common.mobs[i].species() != species || !equine::boost(&mut self.common.mobs[i]) {
            return false;
        }
        let slot = self.common.held_slot();
        let hand = self.common.hand;
        if let Some(stack) = self.common.stack_in(slot).cloned() {
            let (updated, broken) = self.damage_stack(&stack, wear, true);
            if self.common.stack_in(slot) == Some(&stack) {
                *self.common.stack_in_mut(slot) = updated;
                if let Some(broken) = broken {
                    use sunrise_core::equipment_slot::EquipmentSlot;
                    let equipment = match hand {
                        Hand::Main => EquipmentSlot::MainHand,
                        Hand::Off => EquipmentSlot::OffHand,
                    };
                    self.player_equipment_broke(equipment, &broken);
                    self.award_stat("broken", &broken.item, 1);
                    self.player_item_destroyed(&stack, Some(hand));
                    // hurtAndConvertOnBreak keeps the broken stick's components.
                    if self.common.stack_in(slot).is_none() {
                        let mut rod = stack.transmute("minecraft:fishing_rod", 1);
                        rod.damage = 0;
                        *self.common.stack_in_mut(slot) = Some(rod);
                    }
                }
            }
        }
        self.swing_arm();
        true
    }
    /// `AbstractHorse.mobInteract`. Returns whether the click was used.
    pub fn interact_mount(&mut self, index: usize, item: &str) -> bool {
        let held = self.common.held().cloned();
        let m = &self.common.mobs[index];
        let species = m.species();
        // Pigs and striders check occupancy only for boarding, not for saddling.
        if matches!(species, Species::Pig | Species::Strider) {
            return self.interact_steerable(index, item);
        }
        // A server player rides it already (`isVehicle`: no seat, no feeding).
        if self.ridden_by_guest(m.id) {
            return false;
        }
        if sunrise_sim::mobs::llama::is_llama(species) {
            return self.interact_llama(index, item);
        }
        if species == Species::HappyGhast {
            return self.interact_happy_ghast(index, item);
        }
        if sunrise_sim::mobs::nautilus::is_nautilus(species) {
            return self.interact_nautilus(index, item);
        }
        if !equine::is_equine(species) || !m.alive() || Some(m.id) == self.common.riding {
            return false;
        }
        // AbstractHorse.mobInteract: a horse a mob rides (`isVehicle`) has no
        // seat, inventory or feeding to give. Camels seat two and check their
        // passengers themselves.
        let id = m.id;
        if !matches!(species, Species::Camel | Species::CamelHusk)
            && self
                .common
                .mobs
                .iter()
                .any(|p| p.extra.mount == Some(id) && !p.removed && p.alive())
        {
            return false;
        }
        // SkeletonHorse.mobInteract: nothing for an untamed one.
        if species == Species::SkeletonHorse && !m.extra.tame {
            return false;
        }
        let sneaking = self.server_acting_sneak();
        // Horse / ZombieHorse.mobInteract: food is fed, and anything else in
        // hand makes an untamed one mad instead of seating the player.
        let dandelion = [self.common.held(), self.common.offhand.as_ref()]
            .into_iter()
            .flatten()
            .any(|s| &*s.item == "minecraft:golden_dandelion");
        let mut events = Vec::new();
        match equine::held_item(
            &mut self.common.mobs[index],
            item,
            sneaking,
            dandelion,
            &mut events,
        ) {
            Some(equine::HeldItem::Feed) => return self.feed_mount(index, item),
            Some(equine::HeldItem::Refused) => {
                let _ = self.handle_mob_events(events);
                return true;
            }
            None => {}
        }
        let m = &self.common.mobs[index];
        // AbstractHorse.mobInteract: shift + use opens a tamed adult's inventory.
        if !m.is_baby() && m.extra.tame && sneaking {
            self.server_request_mount_menu(index);
            return true;
        }
        if self.feed_mount(index, item) {
            return true;
        }
        let wears_body = held.as_ref().is_some_and(|stack| {
            sunrise_sim::mobs::equip::equippable_in_slot(
                self.items(),
                stack,
                "body",
                sunrise_sim::mobs::species::vanilla_id(species),
            )
        });
        let m = &mut self.common.mobs[index];
        if m.is_baby() {
            return false;
        }
        let at = m.center();
        let tamed = m.extra.tame;
        let sound = if item == "minecraft:saddle" && tamed && !m.extra.saddled() {
            m.set_saddle(held.map(|s| ItemStack { count: 1, ..s }));
            // Camel.getSaddleSound (the equip sound of a camel's saddle).
            Some(match species {
                Species::Camel => "minecraft:entity.camel.saddle",
                Species::CamelHusk => "minecraft:entity.camel_husk.saddle",
                _ => "minecraft:entity.horse.saddle",
            })
        } else if wears_body && tamed && m.extra.body_id().is_none() {
            m.extra
                .set_body_stack(held.map(|s| ItemStack { count: 1, ..s }));
            Some("minecraft:entity.horse.armor")
        } else if item == "minecraft:chest"
            && matches!(species, Species::Donkey | Species::Mule)
            && tamed
            && !m.extra.chested
        {
            m.extra.chested = true;
            // AbstractChestedHorse.playChestEquipsSound, which Mule overrides
            // with its own chest sound.
            Some(match species {
                Species::Mule => "minecraft:entity.mule.chest",
                _ => "minecraft:entity.donkey.chest",
            })
        } else {
            None
        };
        if let Some(sound) = sound {
            let chest = item == "minecraft:chest";
            let m = &mut self.common.mobs[index];
            let silent = m.silent;
            // `LivingEntity.onEquipItem`: a saddle's or armour's equip sound plays
            // at the mount's feet at pitch `1.0F`, seeded with the mount's own
            // random's `nextLong`. `AbstractChestedHorse.playChestEquipsSound`
            // pitches a chest's sound from two floats of that same random, which
            // it draws even while silent, at volume 1.0.
            let pitch = if chest {
                (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0
            } else {
                1.0
            };
            let seed = if chest || silent {
                None
            } else {
                Some(m.random.next_long())
            };
            let feet = [m.position.x, m.position.y, m.position.z];
            if !silent {
                match seed {
                    Some(seed) => self.server_play_sound_seeded(sound, feet, 1.0, pitch, seed),
                    None => self.server_play_sound(sound, feet, 1.0, pitch),
                }
            }
            self.consume_held();
            if !chest {
                let id = self.common.mobs[index].id;
                let pos = self.common.mobs[index].position;
                self.mob_equip_event(id, [pos.x, pos.y, pos.z], true);
            }
            self.common
                .world
                .modified
                .insert((at[0].floor() as i32 >> 4, at[2].floor() as i32 >> 4));
            return true;
        }
        // doPlayerRide: `AbstractHorse.doPlayerRide` turns the rider to the
        // horse's yaw before it seats them.
        let id = self.common.mobs[index].id;
        if !self.native_may_board(id) {
            return true;
        }
        let m = &mut self.common.mobs[index];
        m.extra.flag = false;
        m.extra.counter = 0;
        self.common.yaw = m.yaw;
        self.common.mount.jump_ticks = 0;
        self.common.mount.jump_scale = 0.0;
        self.start_riding(id);
        true
    }
    /// `Pig.mobInteract` / `Strider.mobInteract`: saddle an adult, or ride a saddled mount with no passenger.
    pub(in crate::game::server_side) fn interact_steerable(
        &mut self,
        index: usize,
        item: &str,
    ) -> bool {
        let held = self.common.inventory[self.common.selected].clone();
        let sneaking = self.server_acting_sneak();
        let m = &self.common.mobs[index];
        if m.is_baby() || !m.alive() || Some(m.id) == self.common.riding {
            return false;
        }
        let at = m.center();
        if item == "minecraft:saddle" && !m.extra.saddled() {
            let sound = if m.species() == Species::Strider {
                "minecraft:entity.strider.saddle"
            } else {
                "minecraft:entity.pig.saddle"
            };
            self.common.mobs[index].set_saddle(held.map(|s| ItemStack { count: 1, ..s }));
            self.server_play_sound(sound, at, 1.0, 1.0);
            self.consume_held();
            let id = self.common.mobs[index].id;
            let pos = self.common.mobs[index].position;
            self.mob_equip_event(id, [pos.x, pos.y, pos.z], true);
            self.common
                .world
                .modified
                .insert((at[0].floor() as i32 >> 4, at[2].floor() as i32 >> 4));
            return true;
        }
        // Pig.mobInteract: held food goes to Animal.mobInteract, not a ride.
        let food = m.kind.food.iter().any(|f| f.as_str() == item);
        if !m.extra.saddled() || food || sneaking || self.has_seated_passengers(m.id) {
            return false;
        }
        // `Strider.canAddPassenger`: a strider whose eyes are in lava has no
        // seat, though the click still counts as used.
        if m.species() == Species::Strider && m.fluids.eye_in_lava {
            return true;
        }
        let id = m.id;
        if !self.native_may_board(id) {
            return true;
        }
        self.start_riding(id);
        true
    }
    /// `Entity.interact` / `Mob.canShearEquipment` / `Mob.attemptToShearEquipment`
    /// / `Mob.shearItem`: the saddle of a `#can_equip_saddle` mob comes off
    /// before its mobInteract can ride it, with the saddle's shearing sound.
    /// An occupied mount is not sheared: its mobInteract runs instead.
    pub(in crate::game) fn shear_saddle(&mut self, index: usize) -> Option<bool> {
        let m = &self.common.mobs[index];
        // EntityTypeTags.CAN_EQUIP_SADDLE.
        let saddleable = matches!(
            m.species(),
            Species::Horse
                | Species::SkeletonHorse
                | Species::ZombieHorse
                | Species::Donkey
                | Species::Mule
                | Species::Pig
                | Species::Strider
                | Species::Camel
                | Species::CamelHusk
                | Species::Nautilus
                | Species::ZombieNautilus
        );
        if !self.held_shears(ability::SHEARS_REMOVE_ARMOR)
            || !saddleable
            || m.is_baby()
            || !m.alive()
        {
            return None;
        }
        let id = m.id;
        if self.common.riding == Some(id)
            || self.ridden_by_guest(id)
            || equine::is_vehicle(m)
            || self
                .common
                .mobs
                .iter()
                .any(|r| r.alive() && !r.removed && r.extra.mount == Some(id))
        {
            // !canShearEquipment (isVehicle): the click goes on to mobInteract. A
            // ridden pig or strider has nothing more to give, but a camel takes a
            // second rider and a horse or nautilus opens its inventory.
            return matches!(m.species(), Species::Pig | Species::Strider).then_some(false);
        }
        if self.server_acting_sneak() {
            return None;
        }
        // attemptToShearEquipment takes the body slot before the saddle: horse
        // and nautilus armor come off first (not modelled here).
        if m.extra.body_stack().is_some_and(|body| {
            sunrise_sim::mobs::equip::equippable_in_slot(
                self.items(),
                body,
                "body",
                sunrise_sim::mobs::species::vanilla_id(m.species()),
            )
        }) {
            return None;
        }
        let saddle = m.saddle_item()?;
        if &*saddle.item != "minecraft:saddle" {
            return None;
        }
        let silent = m.silent;
        let at = m.position;
        let offset = sunrise_sim::mobs::riders::passenger_attachment(m)[1];
        self.damage_held_tool();
        self.common.mobs[index].set_saddle(None);
        let context = self.who_context(Some(Who::Player), None);
        self.post_game_event("minecraft:shear", [at.x, at.y, at.z], context);
        self.mob_equip_event(id, [at.x, at.y, at.z], false);
        self.spawn_at_location(at, offset, saddle.clone());
        let target = self.mob_entity(&self.common.mobs[index]);
        self.trigger_item_entity("minecraft:player_sheared_equipment", &saddle, &target);
        // attemptToShearEquipment: playSound(equippable.shearingSound()).
        if !silent {
            self.server_play_sound(
                "minecraft:item.saddle.unequip",
                [at.x, at.y, at.z],
                1.0,
                1.0,
            );
        }
        Some(true)
    }
    /// `AbstractHorse.handleEating`.
    pub(in crate::game::server_side) fn feed_mount(&mut self, index: usize, item: &str) -> bool {
        let Some((heal, age_up, temper, love)) = eating(self.common.mobs[index].species(), item)
        else {
            return false;
        };
        let m = &mut self.common.mobs[index];
        let mut used = false;
        let mut loved = false;
        if love && m.extra.tame && m.extra.age == 0 && m.extra.in_love <= 0 {
            m.extra.in_love = 600;
            m.extra.love_player = true;
            used = true;
            loved = true;
        }
        if m.health < m.attrs.max_health && heal > 0.0 {
            m.heal(
                &self.common.loop_context,
                &|| crate::game::entity_server::hit_reads_of!(self),
                heal,
            );
            used = true;
        }
        if m.is_baby() && age_up > 0 && !m.extra.age_locked {
            // AbstractHorse.handleEating (Camel's too): the HAPPY_VILLAGER's
            // `getRandomX(1.0)`, `getRandomY() + 0.5` and `getRandomZ(1.0)`
            // each draw a double from the mount's own random before it ages
            // up. A server's `Level.addParticle` shows nothing: the client
            // draws its own sparkle (`remote_feed_particles`).
            let (feet, width, height) = (
                [m.position.x, m.position.y, m.position.z],
                f64::from(m.width()),
                f64::from(m.height()),
            );
            sunrise_level::particle_options::around_self(
                &mut m.random,
                feet,
                width,
                height,
                sunrise_level::particle_options::Burst::HORSE_EATING,
                |_, _| {},
            );
            m.extra.age = (m.extra.age + age_up * 20).min(0);
            used = true;
        }
        if temper > 0 && (used || !m.extra.tame) && equine::temper(m) < 100 {
            equine::modify_temper(m, i32::from(temper));
            used = true;
        }
        if !used {
            return false;
        }
        // eating(): the mouth opens and, unless the mount is silent, it eats at
        // its feet, its own random drawing `1.0F + (nextFloat() - nextFloat())
        // * 0.2F` for the pitch.
        m.anim.v[2] = 1.0;
        let at = [m.position.x, m.position.y, m.position.z];
        let id = m.id;
        if !m.silent {
            let sound = match m.species() {
                Species::Camel => "minecraft:entity.camel.eat",
                Species::CamelHusk => "minecraft:entity.camel_husk.eat",
                Species::Donkey => "minecraft:entity.donkey.eat",
                Species::Mule => "minecraft:entity.mule.eat",
                Species::Horse if m.is_baby() => "minecraft:entity.baby_horse.eat",
                // ZombieHorse.getEatingSound.
                Species::ZombieHorse => "minecraft:entity.zombie_horse.eat",
                _ => "minecraft:entity.horse.eat",
            };
            let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0;
            self.server_play_sound(sound, at, 1.0, pitch);
        }
        // gameEvent(GameEvent.EAT): at the mount, from the mount.
        let ctx = self.who_context(Some(Who::Mob(id)), None);
        self.post_game_event("minecraft:eat", at, ctx);
        // Animal.setInLove: entity event 18.
        if loved {
            self.server.love_cause.insert(id, self.server.lan_acting);
            let kind = self.common.mobs[index].kind.id.clone();
            self.native_entity_enter_love_mode(id, &kind, self.server.lan_acting_uuid);
            self.mob_event_particles(id, 18);
        }
        self.consume_held();
        true
    }
    /// Whether the rider's eyes are in water (`Entity.updateFluidOnEyes` then `isEyeInFluid(WATER)`),
    /// and not in a bubble column, as `LivingEntity.baseTick` asks before it lets a rider stay.
    pub fn rider_eyes_in_water(&self) -> bool {
        let Some([x, y, z]) = self.common.eye_position() else {
            return false;
        };
        let (bx, by, bz) = (x.floor() as i32, y.floor() as i32, z.floor() as i32);
        if &*self
            .common
            .res
            .facts()
            .get(self.common.world.block(bx, by, bz))
            .name
            == "minecraft:bubble_column"
        {
            return false;
        }
        self.common
            .fluid_height_of(bx, by, bz, Some(Fluid::Water))
            .is_some_and(|(height, _)| f64::from(by) + f64::from(height) > y)
    }
    /// The rider's input for this tick; returns false when not on a mount.
    pub fn tick_mount_riding(&mut self, keys: Keys) -> bool {
        let Some(i) = self.common.mount_index() else {
            return false;
        };
        // `Player.rideTick`: a sneaking rider gets off (`wantsToStopRiding`), and so does one on a
        // dead mount. `LivingEntity.baseTick`: so does a rider whose eyes are in water on a mount
        // of the `dismounts_underwater` tag (a server's player too: the put-back at the end of `guest_turn` tells their
        // client). When `entity_mount` keeps it on, the mount still reads this tick's input.
        let i = if keys.shift
            || !self.common.mobs[i].alive()
            || (dismounts_underwater(&self.items().entity_type_tags, &self.common.mobs[i])
                && self.rider_eyes_in_water())
        {
            self.dismount_mount();
            let Some(i) = self
                .common
                .mount_index()
                .filter(|&i| self.common.mobs[i].alive())
            else {
                return true;
            };
            i
        } else {
            i
        };
        let strafe = f32::from(u8::from(keys.left)) - f32::from(u8::from(keys.right));
        let forward = f32::from(u8::from(keys.forward)) - f32::from(u8::from(keys.backward));
        // LocalPlayer.aiStep: the riding jump charges while space is held.
        let steer = steering_item(
            self.common.mobs[i].species(),
            self.common.held().map(|s| s.item.as_ref()),
            self.common.offhand.as_ref().map(|s| s.item.as_ref()),
        );
        let jump = self.common.charge_mount_jump(i, keys);
        let m = &mut self.common.mobs[i];
        if jump > 0.0 {
            // AbstractHorse.onPlayerJump: slide on the keys and rear (horses,
            // donkeys and mules).
            if equine::is_equine(m.species()) {
                equine::on_player_jump(m);
            }
            // The nautilus plays its dash sound itself.
            if equine::is_equine(m.species()) {
                // playJumpSound, which the server broadcasts for a guest.
                let sound = format!("minecraft:{}", equine::jump_sound(m));
                let at = m.center();
                self.server_play_sound(&sound, at, 0.4, 1.0);
            }
        }
        let sprint = keys.sprint || self.server_acting_sprint();
        let m = &mut self.common.mobs[i];
        let pending = m.rider.map_or(0.0, |r| r.jump);
        m.rider = Some(equine::Rider {
            strafe,
            forward,
            yaw: self.common.yaw,
            pitch: self.common.pitch,
            jump: if jump > 0.0 { jump } else { pending },
            steer,
            sprint,
            jumping: keys.jump,
            sneak: keys.shift,
            // A server's player (a LAN guest, single player's window) drives
            // a horse from its own client (`handleMoveVehicle`).
            client: self.server.lan_acting.is_some(),
        });
        true
    }
    /// `LivingEntity.baseTick` for the mobs that ride a mount of the `dismounts_underwater` tag (a baby
    /// zombie on a chicken): with their eyes in water they get off it.
    pub fn dismount_underwater_mob_riders(&mut self) {
        let blocked = self.mobs_below_off_level();
        let tags = &self.items().entity_type_tags;
        let swimmers: Vec<u32> = self
            .common
            .mobs
            .iter()
            .filter(|m| !blocked.contains(&m.id) && m.alive() && m.fluids.eye_in_water)
            .filter_map(|m| {
                let mount = m.extra.mount?;
                let under = self.common.mobs.iter().find(|v| v.id == mount)?;
                dismounts_underwater(tags, under).then_some(m.id)
            })
            .collect();
        for rider in swimmers {
            self.dismount_mob(rider);
        }
    }
    /// This player's index in `vehicle`'s `getPassengers`, 0 when it is not listed.
    pub(in crate::game::server_side) fn own_passenger_index(&self, vehicle: u32) -> usize {
        let own = self.riding_player().map(|(id, _)| id);
        self.seated_passengers(vehicle)
            .iter()
            .position(|&(_, id, player)| player && Some(id) == own)
            .unwrap_or(0)
    }
    pub(in crate::game) fn server_own_passenger_index(&self, vehicle: u32) -> usize {
        self.own_passenger_index(vehicle)
    }
    /// `removePassenger` + `getDismountLocationForPassenger`: where the mount
    /// lets the rider out.
    pub(in crate::game::server_side) fn dismount_mount(&mut self) {
        let Some(id) = self.common.riding else {
            return;
        };
        if !self.native_may_leave(id) {
            return;
        }
        self.common.riding = None;
        let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        m.rider = None;
        if m.species() == Species::HappyGhast {
            let id = m.id;
            self.dismount_happy_ghast(id);
            return;
        }
        let mount = Mount::of(m);
        if self.dismount_in_portal(mount.position) {
            return;
        }
        let Some(p) = self.common.player.as_ref() else {
            return;
        };
        let (spot, pose) = self.mount_player_dismount(&mount, self.common.yaw, p);
        if let Some(p) = self.common.player.as_mut() {
            p.position = spot;
            p.velocity = Vec3::ZERO;
            // LivingEntity.setPose: the dismount pose that fit.
            if let Some(pose) = pose
                && let Err(e) = p.set_pose(pose)
            {
                log::warn!("dismount pose: {e}");
            }
        }
    }
    /// Where `mount` lets a player in `p` facing `yaw` off, and the pose they take there.
    pub(in crate::game::server_side) fn mount_player_dismount(
        &self,
        mount: &Mount,
        yaw: f32,
        p: &PlayerMotion,
    ) -> (Vec3, Option<Pose>) {
        let rider = Rider {
            yaw,
            width: p.dimensions.width,
            poses: rider_poses(p),
            main_arm_left: self.server.settings.main_hand_left,
        };
        dismount_location(mount, &rider, &GameDismountWorld(self))
    }

    /// The same player search when the mount is lent out during a mob tick.
    pub(in crate::game::server_side) fn player_dismount_from_lent_mob(
        &self,
        mount: &Mob,
        yaw: f32,
        player: &PlayerMotion,
    ) -> Vec3 {
        if mount.species() == Species::HappyGhast {
            on_top(mount)
        } else {
            self.mount_player_dismount(&Mount::of(mount), yaw, player).0
        }
    }
    /// Where `stop_riding` sets a player in `p` facing `yaw` down off `vehicle`: None when
    /// `vehicle` is no boat, cart or mob.
    pub(in crate::game::server_side) fn player_dismount_point(
        &self,
        vehicle: u32,
        yaw: f32,
        p: &PlayerMotion,
    ) -> Option<Vec3> {
        if let Some(i) = self.common.boats.iter().position(|b| b.id == vehicle) {
            return Some(self.boat_player_dismount_target(i, yaw));
        }
        if let Some(i) = self.common.carts.iter().position(|c| c.id == vehicle) {
            return Some(self.cart_player_dismount_target(i));
        }
        let mount = self.common.mobs.iter().find(|m| m.id == vehicle)?;
        if mount.species() == Species::HappyGhast {
            return Some(on_top(mount));
        }
        Some(self.mount_player_dismount(&Mount::of(mount), yaw, p).0)
    }
    /// `AbstractHorse.tameWithName`: the acting player owns `mob`, then gets
    /// the `tame_animal` trigger.
    pub(in crate::game::server_side) fn tame_mount(&mut self, mob: u32) {
        // RunAroundLikeCrazyGoal.tick: tameWithName by the player on its
        // back, unless AnimalTameEvent is cancelled, which fails the roll
        // instead. Inside a guest's turn the acting ids are the guest's.
        let index = self.common.mobs.iter().position(|m| m.id == mob);
        if let Some(index) = index {
            let (tamer, tamer_entity_id) = (self.native_acting_id(), self.own_entity_id());
            if self.animal_tame_cancelled(index, tamer, tamer_entity_id) {
                let m = &mut self.common.mobs[index];
                let mut events = Vec::new();
                sunrise_sim::mobs::equine::refuse_rider(m, &mut events);
                if let Err(e) = self.handle_mob_events(events) {
                    log::warn!("failed horse taming: {e}");
                }
                return;
            }
        }
        let owner = self.acting_player_uuid();
        if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == mob) {
            // `asks_animal_tame` left the flag for this handler to set.
            m.extra.tame = true;
            m.extra.owner = Some(owner);
            m.persistent = true;
            // AbstractHorse.tameWithName: entity event 7, spawnTamingParticles(true).
            self.mob_event_particles(mob, 7);
        }
        if let Some(view) = self
            .common
            .mobs
            .iter()
            .find(|m| m.id == mob)
            .map(|m| self.mob_entity(m))
        {
            self.trigger_entity("minecraft:tame_animal", "entity", &view);
        }
        if let Some(kind) = self
            .common
            .mobs
            .iter()
            .find(|m| m.id == mob)
            .map(|m| m.kind.id.clone())
        {
            let owner = sunrise_sim::lan::playerdata::ints_uuid(owner);
            self.native_entity_tame(mob, &kind, Some(owner));
        }
    }
    pub(in crate::game::server_side) fn mount_event(&mut self, e: &MobEvent) {
        match *e {
            MobEvent::Tamed { mob } => {
                // tameWithName(player): the rider, a server's player in its turn.
                let rider = self.server.lan.as_ref().and_then(|h| {
                    h.hub
                        .guests
                        .iter()
                        .find(|g| g.vehicle == Some(mob))
                        .map(|g| g.conn)
                });
                if let Some(conn) = rider {
                    self.as_guest(conn, |g| g.tame_mount(mob));
                } else {
                    self.tame_mount(mob);
                }
            }
            MobEvent::Buck { mob } => {
                if self.common.riding == Some(mob) {
                    self.dismount_mount();
                } else {
                    // ejectPassengers: a server's player gets off too
                    // (the put-back at the end of `guest_turn` tells their client).
                    let rider = self.server.lan.as_ref().and_then(|h| {
                        h.hub
                            .guests
                            .iter()
                            .find(|g| g.vehicle == Some(mob))
                            .map(|g| g.conn)
                    });
                    if let Some(conn) = rider {
                        self.as_guest(conn, |g| g.dismount_mount());
                    }
                }
                // RunAroundLikeCrazyGoal.tick: entity event 6, smoke.
                self.mob_event_particles(mob, 6);
            }
            // `Entity.ejectPassengers` before the mob moved off `from`: each
            // rider's `dismountVehicle` reads the vehicle where it stood.
            MobEvent::EjectPassengers { mob, from } => {
                let Some(i) = self.common.mobs.iter().position(|m| m.id == mob) else {
                    return;
                };
                let moved = (self.common.mobs[i].position, self.common.mobs[i].previous);
                self.common.mobs[i].position = from;
                self.common.mobs[i].previous = from;
                if self.common.riding == Some(mob) {
                    self.stop_riding();
                }
                let guests: Vec<u32> = self.server.lan.as_ref().map_or_else(Vec::new, |h| {
                    h.hub
                        .guests
                        .iter()
                        .filter(|g| g.vehicle == Some(mob))
                        .map(|g| g.conn)
                        .collect()
                });
                for conn in guests {
                    self.as_guest(conn, |g| g.stop_riding());
                }
                let riders: Vec<u32> = self
                    .common
                    .mobs
                    .iter()
                    .filter(|m| m.extra.mount == Some(mob))
                    .map(|m| m.id)
                    .collect();
                for rider in riders {
                    self.dismount_mob(rider);
                }
                if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == mob) {
                    (m.position, m.previous) = moved;
                    m.extra.rider = None;
                }
            }
            // `LivingEntity.stabAttack`: `target.stopRiding()`.
            MobEvent::StabDismount { target } => match target {
                Who::Player => self.stop_riding(),
                Who::Mob(id) => self.dismount_mob(id),
            },
            _ => {}
        }
    }
    /// `Entity.propagateFallToPassengers`: whoever rides the mob `mob` takes
    /// `causeFallDamage` of its fall, as the mount's own damage does. The local
    /// player, the server players and the mob on its back each get it, with
    /// their own safe distance and armour.
    pub(in crate::game::server_side) fn fall_to_passengers(
        &mut self,
        mob: u32,
        distance: f64,
        modifier: f32,
    ) {
        let seats = self.seated_passengers(mob);
        for (_, id, player) in seats {
            if player {
                if self.common.riding == Some(mob)
                    && self
                        .common
                        .player
                        .as_ref()
                        .is_some_and(|p| p.entity_id as u32 == id)
                {
                    self.rider_fall_damage(distance, modifier);
                } else if let Some(conn) = self.server.lan.as_ref().and_then(|host| {
                    host.hub
                        .guests
                        .iter()
                        .find(|g| g.entity_id as u32 == id && g.vehicle == Some(mob))
                        .map(|g| g.conn)
                }) {
                    self.guest_hit_by(conn, |g| g.rider_fall_damage(distance, modifier));
                }
                continue;
            }
            let Some(i) = self.common.mob_index.find(&self.common.mobs, id) else {
                continue;
            };
            let join = self.common.mobs[i].join;
            let fall = sunrise_sim::mobs::living::PassengerFall { distance, modifier };
            let mut resume = None;
            let mut events = Vec::new();
            while let Some(i) = self.common.mob_index.find(&self.common.mobs, id) {
                let rider = &mut self.common.mobs[i];
                if rider.join != join
                    || rider.removed
                    || (resume.is_none() && rider.extra.mount != Some(mob))
                {
                    break;
                }
                let level = crate::game::entity_server::damage_level_of!(
                    self,
                    &sunrise_sim::health::NO_ATTACK,
                    no_push,
                    None
                );
                let pause = match resume.take() {
                    None => rider.passenger_fall_until_pause(
                        &self.common.loop_context,
                        level,
                        &mut events,
                        fall,
                    ),
                    Some(pause) => rider.resume_passenger_fall(level, &mut events, pause),
                };
                let Some(pause) = pause else {
                    break;
                };
                if let Some(passengers) = pause.passengers() {
                    self.fall_to_passengers(id, passengers.distance, passengers.modifier);
                } else {
                    self.finish_mob_damage_at(i);
                }
                resume = Some(pause);
            }
            let _ = self.handle_mob_events(events);
        }
    }
    /// The player this game runs (the local one, or the server player it acts
    /// as) takes the fall of the mount under it. `cause_mount_fall` runs the
    /// landing code's own fall damage for the player: creative flight is
    /// exempt, and the statistic, the sounds and the damage records wait in the
    /// player's events for its next tick, as a landing of its own leaves them.
    pub(in crate::game::server_side) fn rider_fall_damage(&mut self, distance: f64, modifier: f32) {
        let Some(player) = self.common.player.as_mut() else {
            return;
        };
        let side = server_game_side!(self);
        let _ = MotionWorld {
            side: &side,
            entities: &[],
            player,
            scene: &mut self.common.world,
            assets: &self.common.pack.assets,
            collision_binding: self.common.collision,
            resources: self.common.motion,
            health: &self.common.health_resources,
            loop_context: &self.common.loop_context,
            damage_hooks: &self.common.damage_hooks,
            tag_items: Some(server_world_items!(self)),
            block_registries: Some(&self.common.pack.block_registries),
        }
        .cause_mount_fall(distance, modifier);
        if self.server.lan_acting.is_some() {
            self.finish_guest_damage();
        }
    }
}

/// The mount a rider gets off: its kind, where it stands, its box,
/// `getBbWidth` and body yaw.
pub struct Mount {
    pub species: Species,
    pub position: Vec3,
    pub bounds: Aabb,
    pub width: f32,
    pub yaw: f32,
}
impl Mount {
    pub(in crate::game::server_side) fn of(m: &Mob) -> Self {
        Self {
            species: m.species(),
            position: m.position,
            bounds: m.bounds(),
            width: m.width(),
            yaw: m.yaw,
        }
    }
}
/// On top of `m`, where it lets off a rider it has no other rule for.
pub(in crate::game) fn on_top(m: &Mob) -> Vec3 {
    Vec3::new(
        m.position.x,
        m.position.y + f64::from(m.height()),
        m.position.z,
    )
}
/// The rider getting off: `getYRot`, `getBbWidth`, and
/// `getLocalBoundsForPose` for each of `Player.getDismountPoses`.
pub struct Rider {
    pub yaw: f32,
    pub width: f32,
    pub poses: Vec<(Pose, Dimensions)>,
    pub main_arm_left: bool,
}
/// `Player.getDismountPoses`: standing, crouching, then crawling.
pub const DISMOUNT_POSES: [Pose; 3] = [Pose::Standing, Pose::Crouching, Pose::Swimming];
/// The rider's box in each dismount pose, at their `minecraft:scale`.
pub(in crate::game::server_side) fn rider_poses(p: &PlayerMotion) -> Vec<(Pose, Dimensions)> {
    // EntityDimensions.scale: every player pose scales alike, so the
    // standing width gives the scale without a mutable attribute lookup.
    let scale = p.dimensions.width / Pose::Standing.dimensions(1.0).width;
    DISMOUNT_POSES
        .iter()
        .map(|&pose| (pose, pose.dimensions(scale)))
        .collect()
}
/// The level queries a dismount makes.
pub trait DismountWorld {
    /// `getFluidState(pos).is(FluidTags.LAVA)`.
    fn lava(&self, p: [i32; 3]) -> bool;
    /// `BlockGetter.getBlockFloorHeight(pos)`.
    fn floor_height(&self, p: [i32; 3]) -> f64;
    /// `DismountHelper.canDismountTo(level, passenger, box)`.
    fn can_dismount_to(&self, bounds: Aabb) -> bool;
}
pub(in crate::game) struct GameDismountWorld<'a, 'g>(
    pub(in crate::game) &'a crate::game::ServerGame<'g>,
);
impl DismountWorld for GameDismountWorld<'_, '_> {
    fn lava(&self, p: [i32; 3]) -> bool {
        let g = self.0;
        g.res.facts().get(g.world.block(p[0], p[1], p[2])).fluid == Fluid::Lava
    }
    fn floor_height(&self, p: [i32; 3]) -> f64 {
        let shape = self.0.block_collision_shape(p);
        let below = std::cell::OnceCell::<VoxelShape>::new();
        sunrise_core::geometry::block_floor_height(&shape, || {
            below.get_or_init(|| self.0.block_collision_shape([p[0], p[1] - 1, p[2]]))
        })
    }
    fn can_dismount_to(&self, bounds: Aabb) -> bool {
        self.0.can_dismount_to(bounds)
    }
}
/// `getDismountLocationForPassenger`, and the pose the rider takes there: each
/// vehicle class's own rule, and `Entity`'s on top of the mount for the rest.
pub fn dismount_location(
    mount: &Mount,
    rider: &Rider,
    world: &impl DismountWorld,
) -> (Vec3, Option<Pose>) {
    if mount.species == Species::Strider {
        return strider_dismount_location(mount, rider, world);
    }
    // `Camel` is an `AbstractHorse`, so the horse rule comes first.
    if equine::is_equine(mount.species) {
        return horse_dismount_location(mount, rider, world);
    }
    if sunrise_sim::mobs::drops::is_animal(mount.species) {
        return animal_dismount_location(mount, rider, world);
    }
    // Entity.getDismountLocationForPassenger: on top of the vehicle's box.
    (
        Vec3::new(
            mount.position.x,
            mount.bounds.max_position().y,
            mount.position.z,
        ),
        None,
    )
}
/// `AbstractHorse.getDismountLocationForPassenger`: out of the horse's box to
/// the main hand's side first, then the other, then its own position.
pub(in crate::game::server_side) fn horse_dismount_location(
    mount: &Mount,
    rider: &Rider,
    world: &impl DismountWorld,
) -> (Vec3, Option<Pose>) {
    // HumanoidArm: the main hand is the right one unless the player set it.
    let main = if rider.main_arm_left { -90.0 } else { 90.0 };
    for turn in [main, -main] {
        let direction = escape_vector(
            f64::from(mount.width),
            f64::from(rider.width),
            mount.yaw + turn,
        );
        if let Some(found) = horse_side(mount, rider, world, direction) {
            return found;
        }
    }
    (mount.position, None)
}
/// `AbstractHorse.getDismountLocationInDirection`: from the horse's feet up to
/// 0.75 above its box, in each of the rider's dismount poses.
pub(in crate::game::server_side) fn horse_side(
    mount: &Mount,
    rider: &Rider,
    world: &impl DismountWorld,
    direction: Vec3,
) -> Option<(Vec3, Option<Pose>)> {
    let (x, z) = (
        mount.position.x + direction.x,
        mount.position.z + direction.z,
    );
    let bottom = mount.bounds.min_position().y.floor() as i32;
    let limit = mount.bounds.max_position().y + 0.75;
    for (pose, dimensions) in &rider.poses {
        let mut y = bottom;
        loop {
            let floor = world.floor_height([x.floor() as i32, y, z.floor() as i32]);
            if f64::from(y) + floor > limit {
                break;
            }
            // DismountHelper.isBlockFloorValid.
            if !floor.is_infinite() && floor < 1.0 {
                let at = Vec3::new(x, f64::from(y) + floor, z);
                if world.can_dismount_to(dimensions.bounds(at)) {
                    return Some((at, Some(*pose)));
                }
            }
            y += 1;
            if f64::from(y) >= limit {
                break;
            }
        }
    }
    None
}
/// `Animal.getDismountLocationForPassenger`: the eight blocks around the
/// mount, its own facing's right side first, at the mount's own height.
pub(in crate::game::server_side) fn animal_dismount_location(
    mount: &Mount,
    rider: &Rider,
    world: &impl DismountWorld,
) -> (Vec3, Option<Pose>) {
    // Entity.getMotionDirection: Direction.fromYRot, in doubles.
    let turn = (f64::from(mount.yaw) / 90.0 + 0.5).floor() as i32;
    let (fx, fz) = [(0, 1), (-1, 0), (0, -1), (1, 0)][(turn & 3) as usize];
    // Direction.getClockWise.
    let (rx, rz) = (-fz, fx);
    // DismountHelper.offsetsForDirection.
    let offsets = [
        (rx, rz),
        (-rx, -rz),
        (rx - fx, rz - fz),
        (-rx - fx, -rz - fz),
        (fx + rx, fz + rz),
        (fx - rx, fz - rz),
        (-fx, -fz),
        (fx, fz),
    ];
    let (bx, by, bz) = (
        mount.position.x.floor() as i32,
        mount.position.y.floor() as i32,
        mount.position.z.floor() as i32,
    );
    for (pose, dimensions) in &rider.poses {
        for (dx, dz) in offsets {
            let p = [bx + dx, by, bz + dz];
            let floor = world.floor_height(p);
            if floor.is_infinite() || floor >= 1.0 {
                continue;
            }
            // Vec3.upFromBottomCenterOf.
            let at = Vec3::new(
                f64::from(p[0]) + 0.5,
                f64::from(p[1]) + floor,
                f64::from(p[2]) + 0.5,
            );
            if world.can_dismount_to(dimensions.bounds(at)) {
                return (at, Some(*pose));
            }
        }
    }
    // `super.getDismountLocationForPassenger`.
    (
        Vec3::new(
            mount.position.x,
            mount.bounds.max_position().y,
            mount.position.z,
        ),
        None,
    )
}
/// `Strider.getDismountLocationForPassenger`: the columns along the rider's
/// facing and 22.5 and 45 degrees either side of it, each from the top of
/// the strider's box down to half a block below its feet. The first block
/// with no lava, a valid floor (`DismountHelper.isBlockFloorValid`) and room
/// for one of the rider's dismount poses (`canDismountTo`) wins, with that
/// pose; without one, on top of the strider with the pose unchanged.
pub(in crate::game::server_side) fn strider_dismount_location(
    strider: &Mount,
    rider: &Rider,
    world: &impl DismountWorld,
) -> (Vec3, Option<Pose>) {
    let (vehicle, passenger) = (f64::from(strider.width), f64::from(rider.width));
    let top = strider.bounds.max_position().y;
    let bottom = strider.bounds.min_position().y - 0.5;
    // A LinkedHashSet: each block once, in the order first reached.
    let mut blocks: Vec<[i32; 3]> = Vec::new();
    for turn in [0.0, -22.5, 22.5, -45.0, 45.0] {
        let offset = escape_vector(vehicle, passenger, rider.yaw + turn);
        let (x, z) = (
            (strider.position.x + offset.x).floor() as i32,
            (strider.position.z + offset.z).floor() as i32,
        );
        let mut y = top;
        let mut block_y = top.floor() as i32;
        while y > bottom {
            if !blocks.contains(&[x, block_y, z]) {
                blocks.push([x, block_y, z]);
            }
            block_y -= 1;
            y -= 1.0;
        }
    }
    for p in blocks {
        if world.lava(p) {
            continue;
        }
        // DismountHelper.isBlockFloorValid.
        let floor = world.floor_height(p);
        if floor.is_infinite() || floor >= 1.0 {
            continue;
        }
        // Vec3.upFromBottomCenterOf.
        let at = Vec3::new(
            f64::from(p[0]) + 0.5,
            f64::from(p[1]) + floor,
            f64::from(p[2]) + 0.5,
        );
        for (pose, dimensions) in &rider.poses {
            if world.can_dismount_to(dimensions.bounds(at)) {
                return (at, Some(*pose));
            }
        }
    }
    (Vec3::new(strider.position.x, top, strider.position.z), None)
}

/// `ServerGamePacketListenerImpl.handlePlayerCommand` START_RIDING_JUMP on the
/// vehicle `m` (`PlayerRideableJumping`): `canJump()`, a scale above zero and
/// then the vehicle's own `handleStartJump`. Returns the sound the server has
/// to play, with its volume — `AbstractHorse.playJumpSound` at 0.4, `Camel`'s
/// dash at 1.0 — or None when the mount takes no such jump or plays its own
/// dash sound (the nautilus does).
///
/// Nothing here moves the mount on its own: what a horse, camel or nautilus
/// does with the jump is its `handleStartJump`, and the travel that follows
/// (a dash, a rear) is the mount's own tick.
pub(in crate::game) fn start_riding_jump(
    m: &mut sunrise_sim::mobs::Mob,
    data: i32,
) -> Option<(&'static str, f32)> {
    // PlayerRideableJumping.canJump: AbstractHorse.isSaddled, and a camel that
    // is not refusing to move keeps its own gate.
    if data <= 0 || !m.extra.saddled() {
        return None;
    }
    if sunrise_sim::mobs::camel::is_camel(m.species()) {
        if sunrise_sim::mobs::camel::refuse_to_move(m) {
            return None;
        }
        // Camel.handleStartJump: the dash sound, the entity action and DASH.
        sunrise_sim::mobs::camel::set_dashing(m, true);
        Some((sunrise_sim::mobs::camel::voice(m, 3), 1.0))
    } else if sunrise_sim::mobs::nautilus::is_nautilus(m.species()) {
        // AbstractNautilus.handleStartJump: DASH, and the dash sound it makes
        // itself when the dash runs. The momentum is `onPlayerJump`'s pending
        // scale, which the nautilus spends on its next travel; a guest's ride
        // tick fills `Rider::jump` from the keys the host hears, so the scale
        // its own bar charged comes from this packet.
        sunrise_sim::mobs::nautilus::on_player_jump(m, data);
        None
    } else if equine::is_equine(m.species()) {
        // AbstractHorse.handleStartJump, which is onPlayerJump for the slide
        // and the rear, then playJumpSound.
        equine::on_player_jump(m);
        Some((equine::jump_sound(m), 0.4))
    } else {
        None
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    #[cfg(test)]
    /// A lava sea up to y 30 with full blocks placed in it; air above.
    pub(in crate::game) struct Sea {
        pub(in crate::game) blocks: HashMap<[i32; 3], ()>,
    }
    #[cfg(test)]
    impl Sea {
        pub(in crate::game::server_side) fn with(blocks: &[[i32; 3]]) -> Self {
            Self {
                blocks: blocks.iter().map(|&b| (b, ())).collect(),
            }
        }
        pub(in crate::game::server_side) fn solid(&self, p: [i32; 3]) -> bool {
            self.blocks.contains_key(&p)
        }
    }
    #[cfg(test)]
    impl DismountWorld for Sea {
        fn lava(&self, p: [i32; 3]) -> bool {
            p[1] <= 30 && !self.solid(p)
        }
        fn floor_height(&self, p: [i32; 3]) -> f64 {
            if self.solid(p) {
                1.0
            } else if self.solid([p[0], p[1] - 1, p[2]]) {
                0.0
            } else {
                f64::NEG_INFINITY
            }
        }
        fn can_dismount_to(&self, bounds: Aabb) -> bool {
            let (lo, hi) = (bounds.min_position(), bounds.max_position());
            let cells = |a: f64, b: f64| a.floor() as i32..=(b - 1.0e-7).floor() as i32;
            !cells(lo.x, hi.x).any(|x| {
                cells(lo.y, hi.y).any(|y| cells(lo.z, hi.z).any(|z| self.solid([x, y, z])))
            })
        }
    }
    /// The top of a lava source at y 30 (`FlowingFluid.getOwnHeight`, 8/9).
    pub const SURFACE: f64 = 30.0 + 8.0 / 9.0;
    pub fn rider(yaw: f32) -> Rider {
        Rider {
            yaw,
            width: 0.6,
            poses: DISMOUNT_POSES
                .iter()
                .map(|&pose| (pose, pose.dimensions(1.0)))
                .collect(),
            main_arm_left: false,
        }
    }
    /// What one feeding made the level show.
    #[derive(Debug)]
    pub struct Shown {
        pub sounds: Vec<(String, [f64; 3], f32, f32)>,
        pub events: Vec<(&'static str, [f64; 3], Option<Who>)>,
    }

    #[cfg(test)]
    use sunrise_gameplay_common::game::server_side::mounts::tests::block_of;

    #[cfg(test)]
    use sunrise_gameplay_common::game::server_side::mounts::tests::mob;

    use super::*;
    #[cfg(test)]
    use std::collections::HashMap;

    #[cfg(test)]
    /// `ZombieHorse.isFood` is `#minecraft:zombie_horse_food`, which holds only
    /// the red mushroom, and `AbstractHorse.isFood` is `#minecraft:horse_food`,
    /// which holds none (#8593). A skeleton horse inherits the horse food tag.
    #[test]
    fn only_a_zombie_horse_eats_a_red_mushroom() {
        assert_eq!(
            eating(Species::ZombieHorse, "minecraft:red_mushroom"),
            Some((3.0, 0, 3, false)),
            "a zombie horse takes the red mushroom"
        );
        for item in ["minecraft:wheat", "minecraft:apple", "minecraft:hay_block"] {
            assert_eq!(eating(Species::ZombieHorse, item), None, "{item}");
        }
        for species in [
            Species::Horse,
            Species::Donkey,
            Species::Mule,
            Species::SkeletonHorse,
        ] {
            assert_eq!(
                eating(species, "minecraft:red_mushroom"),
                None,
                "{species:?}: horse food has no red mushrooms"
            );
            assert_eq!(
                eating(species, "minecraft:wheat"),
                Some((2.0, 20, 3, false)),
                "{species:?}: wheat is horse food"
            );
        }
        assert_eq!(
            eating(Species::Camel, "minecraft:cactus"),
            Some((2.0, 10, 0, true))
        );
        assert_eq!(
            eating(Species::CamelHusk, "minecraft:rabbit_foot"),
            Some((2.0, 10, 0, false))
        );
    }

    #[cfg(test)]
    /// `handlePlayerCommand` START_RIDING_JUMP reaches the ridden vehicle's
    /// `handleStartJump`: a saddled horse rears, slides on the rider's keys and
    /// its jump sound is played at 0.4; a donkey and a mule have their own
    /// sound; a camel dashes with its dash sound at 1.0. An unsaddled mount
    /// (`canJump` false) or a scale of zero takes nothing.
    #[test]
    fn a_released_jump_bar_starts_the_mounts_own_jump() {
        let Some(mut horse) = mob("minecraft:horse") else {
            return;
        };
        // Unsaddled: AbstractHorse.canJump is isSaddled.
        assert_eq!(start_riding_jump(&mut horse, 100), None);
        assert_eq!(horse.extra.counter, 0, "no rear without a jump");
        assert!(!horse.extra.stand_sliding);
        horse.set_saddle(Some(sunrise_items::items::ItemStack::new(
            "minecraft:saddle",
            1,
        )));
        // A scale of zero is not a jump (`data > 0`).
        assert_eq!(start_riding_jump(&mut horse, 0), None);
        assert_eq!(horse.extra.counter, 0);
        assert_eq!(
            start_riding_jump(&mut horse, 100),
            Some(("entity.horse.jump", 0.4))
        );
        assert_eq!(
            horse.extra.counter, 20,
            "standIfPossible rears for 20 ticks"
        );
        assert!(horse.extra.stand_sliding, "onPlayerJump slides on the keys");
        for (kind, sound) in [
            ("minecraft:donkey", "entity.donkey.jump"),
            ("minecraft:mule", "entity.mule.jump"),
        ] {
            let Some(mut m) = mob(kind) else {
                continue;
            };
            m.set_saddle(Some(sunrise_items::items::ItemStack::new(
                "minecraft:saddle",
                1,
            )));
            assert_eq!(start_riding_jump(&mut m, 55), Some((sound, 0.4)), "{kind}");
        }
        // A skeleton horse in water splashes instead.
        let Some(mut skeleton) = mob("minecraft:skeleton_horse") else {
            return;
        };
        skeleton.set_saddle(Some(sunrise_items::items::ItemStack::new(
            "minecraft:saddle",
            1,
        )));
        skeleton.fluids.in_water = true;
        assert_eq!(
            start_riding_jump(&mut skeleton, 100),
            Some(("entity.skeleton_horse.jump_water", 0.4))
        );
    }

    #[cfg(test)]
    /// `Camel.handleStartJump`: the dash sound, then DASH, which starts the
    /// cooldown; a sitting camel refuses to move and cannot jump
    /// (`Camel.canJump`).
    #[test]
    fn a_released_jump_bar_starts_a_camels_dash() {
        let Some(mut camel) = mob("minecraft:camel") else {
            return;
        };
        assert_eq!(start_riding_jump(&mut camel, 100), None, "no saddle");
        camel.set_saddle(Some(sunrise_items::items::ItemStack::new(
            "minecraft:saddle",
            1,
        )));
        assert_eq!(
            start_riding_jump(&mut camel, 100),
            Some(("entity.camel.dash", 1.0))
        );
        assert!(camel.extra.flag2, "Camel.setDashing(true)");
        assert_eq!(
            camel.extra.anim_ticks, 0,
            "an age-0 camel starts no cooldown"
        );
        // Sitting out the ride: refuseToMove keeps the dash from starting.
        camel.extra.camel_sitting = true;
        camel.extra.flag2 = false;
        assert_eq!(start_riding_jump(&mut camel, 100), None);
        assert!(!camel.extra.flag2);
    }

    #[cfg(test)]
    /// `AbstractNautilus.onPlayerJump` through `handlePlayerCommand`
    /// START_RIDING_JUMP: the scale the rider's own bar charged reaches the
    /// mount, and its next travel dashes with it. A nautilus in its dash
    /// cooldown takes no jump (the pending scale is left alone), and there is
    /// no sound for the server to play: the nautilus plays its dash sound
    /// itself when the dash runs.
    #[test]
    fn a_guests_released_jump_bar_arms_the_nautilus_dash() {
        let Some(mut m) = mob("minecraft:nautilus") else {
            return;
        };
        m.set_saddle(Some(sunrise_items::items::ItemStack::new(
            "minecraft:saddle",
            1,
        )));
        // The guest's ride tick fills the rider; the packet arrives after it.
        m.rider = Some(equine::Rider::default());
        assert_eq!(start_riding_jump(&mut m, 75), None);
        let jump = m.rider.expect("the rider stays on").jump;
        // 0.4 + 0.4 * 75 / 90: a three-quarter bar.
        assert!((jump - 0.733_333_3).abs() < 1.0e-6, "{jump}");
        // A full bar is a scale of 1.
        assert_eq!(start_riding_jump(&mut m, 100), None);
        assert_eq!(m.rider.expect("the rider stays on").jump, 1.0);
        // The dash clock: a dash still cooling down refuses the jump, so the
        // scale already on the rider stays.
        m.extra.anim_ticks = 40;
        assert_eq!(start_riding_jump(&mut m, 30), None);
        assert_eq!(m.rider.expect("the rider stays on").jump, 1.0);
        // An unsaddled nautilus cannot jump at all (`AbstractNautilus.canJump`).
        m.extra.set_saddled(false);
        m.extra.anim_ticks = 0;
        assert_eq!(start_riding_jump(&mut m, 100), None);
        assert_eq!(m.rider.expect("the rider stays on").jump, 1.0);
        // The scale is spent by the mount's next travel: `executeRidersJump`
        // pushes along the rider's look, starts the 40-tick dash clock, raises
        // DASH and queues the dash sound.
        m.extra.set_saddled(true);
        equine::ridden_step(
            &mut m,
            &sunrise_gameplay_common::game::test_server_load()
                .loop_context(sunrise_world::native_load_context::LoopSide::Server, []),
        );
        assert_eq!(m.rider.expect("the rider stays on").jump, 0.0);
        assert_eq!(m.extra.anim_ticks, 40, "the dash clock");
        assert!(m.extra.flag2, "DASH");
        assert_eq!(m.extra.scratch[1], 1, "the dash sound plays next tick");
        assert!(
            m.velocity.z > 0.0 && m.velocity.x.abs() < 1.0e-9,
            "the dash goes along the rider's look (+z): {:?}",
            m.velocity
        );
    }

    #[cfg(test)]
    #[test]
    fn steering_accepts_the_correct_item_in_either_hand() {
        let carrot = "minecraft:carrot_on_a_stick";
        let fungus = "minecraft:warped_fungus_on_a_stick";
        assert!(steering_item(Species::Pig, Some(carrot), None));
        assert!(steering_item(Species::Pig, None, Some(carrot)));
        assert!(steering_item(Species::Strider, None, Some(fungus)));
        assert!(!steering_item(Species::Pig, Some(fungus), None));
        assert!(!steering_item(Species::Horse, Some(carrot), Some(fungus)));
    }

    #[cfg(test)]
    /// A strider (0.9 by 1.7) standing on the lava at x, z, facing south.
    pub(in crate::game::server_side) fn strider_at(x: f64, z: f64) -> Mount {
        Mount {
            species: Species::Strider,
            position: Vec3::new(x, SURFACE, z),
            bounds: Aabb::new(
                Vec3::new(x - 0.45, SURFACE, z - 0.45),
                Vec3::new(x + 0.45, SURFACE + 1.7, z + 0.45),
            ),
            width: 0.9,
            yaw: 0.0,
        }
    }

    #[cfg(test)]
    #[test]
    fn a_strider_in_open_lava_puts_its_rider_on_its_back() {
        // #3513: getting off in a lava sea used to put the player beside the
        // strider, in the lava; with no floor in reach vanilla puts the rider
        // on top of the strider instead.
        let sea = Sea::with(&[]);
        let (at, pose) = dismount_location(&strider_at(0.5, 0.5), &rider(0.0), &sea);
        assert!(!sea.lava(block_of(at)), "dismounted into lava at {at:?}");
        assert_eq!(at, Vec3::new(0.5, SURFACE + 1.7, 0.5));
        assert_eq!(pose, None);
    }

    #[cfg(test)]
    #[test]
    fn a_strider_by_the_shore_puts_its_rider_on_the_shore() {
        // Netherrack 45 degrees to the right of a rider facing south (+z);
        // the straight-ahead and 22.5 degree columns are lava.
        let sea = Sea::with(&[[-1, 30, 1]]);
        let (at, pose) = dismount_location(&strider_at(0.5, 0.5), &rider(0.0), &sea);
        assert_eq!(at, Vec3::new(-0.5, 31.0, 1.5));
        assert_eq!(pose, Some(Pose::Standing));
    }

    #[cfg(test)]
    #[test]
    fn a_low_ceiling_on_the_shore_makes_the_rider_crawl_off() {
        let sea = Sea::with(&[[0, 30, 1], [0, 32, 1]]);
        let (at, pose) = dismount_location(&strider_at(0.5, 0.5), &rider(0.0), &sea);
        assert_eq!(at, Vec3::new(0.5, 31.0, 1.5));
        assert_eq!(pose, Some(Pose::Swimming));
    }

    /// Feeds `item` to mob 0 and collects the sounds and the `minecraft:eat`
    /// game events it made.
    pub fn feed(game: &mut crate::game::ServerGame<'_>, item: &str) -> (bool, Shown) {
        game.common.inventory[game.common.selected] = Some(ItemStack::new(item, 2));
        game.start_audio_capture();
        crate::game::sculk_server::TEST_POSTED_EVENTS
            .with(|events| *events.borrow_mut() = Some(Vec::new()));
        let used = game.interact_mount(0, item);
        let sounds = std::mem::take(&mut game.server.pending_sounds)
            .into_iter()
            .map(|s| (s.event, s.position, s.volume, s.pitch))
            .collect();
        game.stop_audio_capture();
        let events = crate::game::sculk_server::TEST_POSTED_EVENTS.with(|events| {
            events
                .borrow_mut()
                .take()
                .unwrap_or_default()
                .into_iter()
                .filter(|(event, _, _)| *event == "minecraft:eat")
                .collect()
        });
        (used, Shown { sounds, events })
    }

    /// A whole, tamed mount of `kind` standing at `at` with `seed` for its own
    /// random, alone in `game`.
    pub fn mount(
        game: &mut crate::game::ServerGame<'_>,
        kind: &str,
        id: u32,
        at: [f64; 3],
        seed: i64,
    ) {
        let kind = game.res.mob_types().get(kind).unwrap().clone();
        let mut m = sunrise_sim::mobs::Mob::new(id, kind, Vec3::new(at[0], at[1], at[2]), 0.0);
        m.extra.tame = true;
        m.health = m.attrs.max_health;
        m.random = sunrise_core::random::LegacyRandom::new(seed);
        game.mobs.clear();
        game.mobs.push(m);
    }

    /// Equips `item` on mob 0 and collects the sounds and the `minecraft:equip`
    /// game events it made.
    pub fn equip(game: &mut crate::game::ServerGame<'_>, item: &str) -> (bool, Shown) {
        game.common.inventory[game.common.selected] = Some(ItemStack::new(item, 2));
        game.start_audio_capture();
        crate::game::sculk_server::TEST_POSTED_EVENTS
            .with(|events| *events.borrow_mut() = Some(Vec::new()));
        let used = game.interact_mount(0, item);
        let sounds = std::mem::take(&mut game.server.pending_sounds)
            .into_iter()
            .map(|s| (s.event, s.position, s.volume, s.pitch))
            .collect();
        game.stop_audio_capture();
        let events = crate::game::sculk_server::TEST_POSTED_EVENTS.with(|events| {
            events
                .borrow_mut()
                .take()
                .unwrap_or_default()
                .into_iter()
                .filter(|(event, _, _)| *event == "minecraft:equip")
                .collect()
        });
        (used, Shown { sounds, events })
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::passengers_common::PassengersCommonGameExt2 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

use sunrise_gameplay_common::game::minecarts::CartDismountGameExt as _;
