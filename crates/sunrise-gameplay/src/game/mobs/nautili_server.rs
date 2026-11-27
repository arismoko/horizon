use crate::game::ServerGame;
use sunrise_core::random::RandomSource;
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::{equip::equippable_in_slot, nautilus};

impl ServerGame<'_> {
    /// `AbstractNautilus.mobInteract`. Returns whether the click was used.
    pub fn interact_nautilus(&mut self, index: usize, item: &str) -> bool {
        let m = &self.common.mobs[index];
        if !m.alive() || Some(m.id) == self.common.riding {
            return false;
        }
        let taming: Vec<String> = self
            .common
            .res
            .items()
            .item_tags
            .get("minecraft:nautilus_taming_items")
            .map(|t| t.iter().cloned().collect())
            .unwrap_or_default();
        let food = nautilus::is_food(m, item, &taming);
        let sneaking = self.server_acting_sneak();
        let tame = m.extra.tame;
        if m.is_baby() {
            // Animal.mobInteract: food grows a baby up by a tenth.
            if !food {
                return false;
            }
            let m = &mut self.common.mobs[index];
            if !m.extra.age_locked {
                let secs = ((-m.extra.age / 20) as f32 * 0.1) as i32;
                sunrise_sim::mobs::ai::age_up(m, secs, true);
            }
            let sound = nautilus::eat_sound(m);
            self.use_nautilus_food(item);
            self.nautilus_sound(index, sound);
            return true;
        }
        // AbstractNautilus.mobInteract: shift + use on a tame adult opens its
        // saddle and armour screen (openCustomInventoryScreen).
        if tame && sneaking {
            self.server_request_mount_menu(index);
            return true;
        }
        if !item.is_empty() {
            if !tame && food {
                // tryToTame: one time in three.
                self.use_nautilus_food(item);
                let rolled = self.common.mobs[index]
                    .random
                    .next_int_bound(3)
                    .unwrap_or(1)
                    == 0;
                let acting = self.acting_player_uuid();
                let (tamer, tamer_entity_id) = (self.native_acting_id(), self.own_entity_id());
                let tamed = rolled && !self.animal_tame_cancelled(index, tamer, tamer_entity_id);
                let m = &mut self.common.mobs[index];
                if tamed {
                    m.extra.tame = true;
                    m.extra.owner = Some(acting);
                    m.persistent = true;
                    m.nav_stop();
                }
                if tamed {
                    self.set_event_ai_target(
                        self.common.mobs[index].id,
                        None,
                        self.server.lan_acting,
                    );
                }
                let sound = nautilus::eat_sound(&self.common.mobs[index]);
                // TamableAnimal.spawnTamingParticles (entity event 7 or 6).
                self.mob_event_particles(self.common.mobs[index].id, if tamed { 7 } else { 6 });
                self.nautilus_sound(index, sound);
                if tamed {
                    let view = self.mob_entity(&self.common.mobs[index]);
                    self.trigger_entity("minecraft:tame_animal", "entity", &view);
                    self.pet_tamed(index);
                }
                return true;
            }
            let m = &self.common.mobs[index];
            if food && m.health < m.attrs.max_health {
                // feed(player, hand, stack, 2, 1).
                let sound = nautilus::eat_sound(m);
                // TamableAnimal.feed: heals 2 x the food's nutrition, 1 for an
                // item with no food component.
                let nutrition = self
                    .common
                    .res
                    .items()
                    .get(item)
                    .and_then(|i| i.food.as_ref())
                    .map(|f| f.0 as i32);
                let m = &mut self.common.mobs[index];
                m.heal(
                    &self.common.loop_context,
                    &|| crate::game::entity_server::hit_reads_of!(self),
                    nautilus::feed_heal(nutrition),
                );
                self.use_nautilus_food(item);
                self.nautilus_sound(index, sound);
                return true;
            }
            // Equippable.equipOnInteract: a saddle or nautilus armour on a
            // tame adult.
            let wears_body = self.common.inventory[self.common.selected]
                .as_ref()
                .is_some_and(|stack| {
                    equippable_in_slot(
                        self.items(),
                        stack,
                        "body",
                        sunrise_sim::mobs::species::vanilla_id(self.common.mobs[index].species()),
                    )
                });
            let m = &mut self.common.mobs[index];
            if item == "minecraft:saddle" && tame && !m.extra.saddled() {
                let held = self.common.inventory[self.common.selected].clone();
                m.set_saddle(held.map(|s| ItemStack { count: 1, ..s }));
                let sound = nautilus::saddle_sound(m);
                self.nautilus_sound(index, sound);
                self.consume_held();
                let id = self.common.mobs[index].id;
                let pos = self.common.mobs[index].position;
                self.mob_equip_event(id, [pos.x, pos.y, pos.z], true);
                return true;
            }
            if wears_body && tame && m.extra.body_id().is_none() {
                let held = self.common.inventory[self.common.selected].clone();
                m.extra
                    .set_body_stack(held.map(|s| ItemStack { count: 1, ..s }));
                self.nautilus_sound(index, "item.armor.equip_nautilus");
                self.consume_held();
                let id = self.common.mobs[index].id;
                let pos = self.common.mobs[index].position;
                self.mob_equip_event(id, [pos.x, pos.y, pos.z], true);
                return true;
            }
        }
        if tame && !food {
            // doPlayerRide: `player.startRiding(this)`, which keeps the
            // rider's view (`ServerPlayer.startRiding`'s `Relative.ROTATION`).
            let id = self.common.mobs[index].id;
            if !self.native_may_board(id) {
                return true;
            }
            self.break_leash(id, true, false);
            self.common.mount.reset_jump();
            self.start_riding(id);
            return true;
        }
        // Animal.mobInteract: a tame adult fed at full health falls in love
        // (a zombie nautilus too: `ZombieNautilus` overrides neither `isFood`
        // nor `mobInteract`, and its brain simply has no make-love goal).
        let m = &mut self.common.mobs[index];
        if food && m.extra.age == 0 && m.extra.in_love <= 0 {
            m.extra.in_love = 600;
            m.extra.love_player = true;
            let sound = nautilus::eat_sound(m);
            let id = m.id;
            self.server.love_cause.insert(id, self.server.lan_acting);
            let kind = self.common.mobs[index].kind.id.clone();
            self.native_entity_enter_love_mode(id, &kind, self.server.lan_acting_uuid);
            self.use_nautilus_food(item);
            self.nautilus_sound(index, sound);
            // Animal.setInLove: entity event 18.
            self.mob_event_particles(self.common.mobs[index].id, 18);
            return true;
        }
        false
    }
    pub(in crate::game::server_side) fn item_tagged(&self, tag: &str, item: &str) -> bool {
        self.common
            .res
            .items()
            .item_tags
            .get(tag)
            .is_some_and(|t| t.contains(item))
    }
    /// `AbstractNautilus.interact` (`super.interact` is `Mob.interact`): any
    /// click on a nautilus, used or not, makes it persistent before its own
    /// handling runs.
    pub(in crate::game::server_side) fn nautilus_clicked(&mut self, id: Option<i32>) {
        let Some(index) = id.and_then(|id| {
            sunrise_gameplay_common::game::item_actions::named_entity(&self.common.mobs, id, |m| {
                m.id
            })
        }) else {
            return;
        };
        if nautilus::is_nautilus(self.common.mobs[index].species()) {
            self.common.mobs[index].persistent = true;
        }
    }
    pub(in crate::game::server_side) fn nautilus_sound(&mut self, index: usize, sound: &str) {
        let at = self.common.mobs[index].center();
        self.server_play_sound(&format!("minecraft:{sound}"), at, 1.0, 1.0);
    }
    /// `AbstractNautilus.usePlayerItem`: bucketed fish leave a water bucket.
    pub(in crate::game::server_side) fn use_nautilus_food(&mut self, item: &str) {
        let bucket = self.item_tagged("minecraft:nautilus_bucket_food", item);
        self.consume_held();
        if bucket && !self.common.creative {
            let water = ItemStack::new("minecraft:water_bucket", 1);
            if self.common.inventory[self.common.selected].is_none() {
                self.common.inventory[self.common.selected] = Some(water);
            } else if let Some(rest) = self.add_stack_to_inventory(water) {
                let at = self.common.eye_position().unwrap_or([0.0; 3]);
                self.spawn_item(rest, at, [0.0; 3], 40);
            }
        }
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

use sunrise_gameplay_common::game::tables::GameItems as _;
