//! Llamas on the game side: `AbstractChestedHorse.mobInteract` and
//! `Llama.handleEating` (wheat and hay), chests, carpets, shearing a carpet
//! off and riding.
use crate::game::ServerGame;
use sunrise_core::random::RandomSource;
use sunrise_gameplay_common::game::item_abilities::ability;
use sunrise_sim::mobs::{equine, llama};

impl ServerGame<'_> {
    /// `Llama.handleEating`: wheat and hay bales heal, age up a baby, raise
    /// the temper and, from a hay bale, put a tame adult in love.
    pub(in crate::game::server_side) fn feed_llama(&mut self, index: usize, item: &str) -> bool {
        let (age_up, temper, heal) = match item {
            "minecraft:wheat" => (10, 3, 2.0),
            "minecraft:hay_block" => (90, 6, 10.0),
            _ => return false,
        };
        let m = &mut self.common.mobs[index];
        let mut used = false;
        let mut loved = false;
        if item == "minecraft:hay_block" && m.extra.tame && m.extra.age == 0 && m.extra.in_love <= 0
        {
            m.extra.in_love = 600;
            m.extra.love_player = true;
            used = true;
            loved = true;
        }
        if m.health < m.attrs.max_health {
            m.heal(
                &self.common.loop_context,
                &|| crate::game::entity_server::hit_reads_of!(self),
                heal,
            );
            used = true;
        }
        let mut happy = false;
        if m.is_baby() && !m.extra.age_locked {
            m.extra.age = (m.extra.age + age_up * 20).min(0);
            used = true;
            happy = true;
        }
        if (used || !m.extra.tame) && equine::temper(m) < llama::MAX_TEMPER {
            equine::modify_temper(m, temper);
            used = true;
        }
        if happy {
            // AbstractHorse.handleEating: one at rest about the body.
            let id = m.id;
            self.common.particles_around_mob(
                id,
                "happy_villager",
                sunrise_level::particle_options::Burst::HORSE_EATING,
            );
        }
        if !used {
            return false;
        }
        // Animal.setInLove: entity event 18.
        if loved {
            let id = self.common.mobs[index].id;
            self.server.love_cause.insert(id, self.server.lan_acting);
            let kind = self.common.mobs[index].kind.id.clone();
            self.native_entity_enter_love_mode(id, &kind, self.server.lan_acting_uuid);
            self.mob_event_particles(id, 18);
        }
        self.llama_sound(index, "entity.llama.eat", 1.0);
        self.consume_held();
        true
    }
    pub(in crate::game::server_side) fn is_led_by_trader(&self, index: usize) -> bool {
        let m = &self.common.mobs[index];
        m.species() == sunrise_sim::mobs::Species::TraderLlama
            && matches!(
                m.extra.leash,
                Some(sunrise_sim::mobs::leash::Holder::Mob(id))
                    if self.mobs.iter().any(|o| o.id == id
                        && o.species() == sunrise_sim::mobs::Species::WanderingTrader)
            )
    }
}

impl ServerGame<'_> {
    /// `AbstractChestedHorse.mobInteract` and `AbstractHorse.mobInteract` for
    /// llamas and trader llamas. Returns whether the click was used.
    pub fn interact_llama(&mut self, index: usize, item: &str) -> bool {
        let m = &self.common.mobs[index];
        if !m.alive() || Some(m.id) == self.common.riding {
            return false;
        }
        let sneaking = self.server_acting_sneak();
        let (baby, tamed, vehicle) = (m.is_baby(), m.extra.tame, m.rider.is_some());
        let open_inventory = !baby && tamed && sneaking;
        if !vehicle && !open_inventory && !item.is_empty() {
            if self.common.mobs[index].kind.food.iter().any(|f| f == item) {
                return self.feed_llama(index, item);
            }
            if !tamed {
                let mut events = Vec::new();
                equine::make_mad(&mut self.common.mobs[index], &mut events);
                let _ = self.handle_mob_events(events);
                return true;
            }
            if !self.common.mobs[index].extra.chested && item == "minecraft:chest" {
                self.common.mobs[index].extra.chested = true;
                self.llama_sound(index, "entity.llama.chest", 1.0);
                self.consume_held();
                return true;
            }
        }
        // AbstractHorse.mobInteract.
        if vehicle || baby {
            return false;
        }
        if tamed && sneaking {
            // AbstractHorse.openCustomInventoryScreen.
            self.server_request_mount_menu(index);
            return true;
        }
        let m = &self.common.mobs[index];
        let carpet = |i: &str| i.ends_with("_carpet") && i.starts_with("minecraft:");
        if !item.is_empty() {
            if self.held_shears(ability::SHEARS_REMOVE_ARMOR)
                && m.extra.body_id().is_some_and(carpet)
            {
                // Equippable.canBeSheared: the carpet comes off.
                let at = m.center();
                let carpet = self.common.mobs[index].take_body_equipment().unwrap();
                self.server_play_sound("minecraft:item.llama_carpet.unequip", at, 1.0, 1.0);
                self.spawn_item(carpet, at, [0.0, 0.2, 0.0], 10);
                self.damage_held_tool();
                let id = self.common.mobs[index].id;
                let pos = self.common.mobs[index].position;
                self.mob_equip_event(id, [pos.x, pos.y, pos.z], false);
                return true;
            }
            if carpet(item) && m.extra.body_id().is_none() {
                let held = self.common.inventory[self.common.selected]
                    .clone()
                    .map(|mut stack| {
                        stack.count = 1;
                        stack
                    });
                self.common.mobs[index].extra.set_body_stack(held);
                self.llama_sound(index, "entity.llama.swag", 1.0);
                self.consume_held();
                let id = self.common.mobs[index].id;
                let pos = self.common.mobs[index].position;
                self.mob_equip_event(id, [pos.x, pos.y, pos.z], true);
                return true;
            }
        }
        // TraderLlama.doPlayerRide: not while a wandering trader leads it.
        if self.is_led_by_trader(index) {
            return true;
        }
        // doPlayerRide; mounting drops a lead.
        let id = self.common.mobs[index].id;
        if !self.native_may_board(id) {
            return true;
        }
        self.break_leash(id, true, false);
        let m = &mut self.common.mobs[index];
        m.extra.flag = false;
        m.extra.counter = 0;
        self.common.yaw = m.yaw;
        self.common.mount.reset_jump();
        self.start_riding(id);
        true
    }
    pub(in crate::game::server_side) fn llama_sound(
        &mut self,
        index: usize,
        sound: &str,
        volume: f32,
    ) {
        let at = self.common.mobs[index].center();
        let pitch = (self.common.world.random.next_float() - self.common.world.random.next_float())
            * 0.2
            + 1.0;
        self.server_play_sound(&format!("minecraft:{sound}"), at, volume, pitch);
    }
}

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;
