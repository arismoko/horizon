//! Zombie villagers in the world: zombies turning the villagers they kill
//! into zombie villagers (`Zombie.killedEntity`), the golden apple that starts
//! a cure (`ZombieVillager.mobInteract`), and the cured villager that
//! remembers who saved it (`ZombieVillager.finishConversion`).
use super::commands::Target;
use sunrise_core::random::RandomSource;
use sunrise_items::items::ItemStack;
use sunrise_level::effects::EffectInstance;
use sunrise_sim::{
    advancements as adv,
    gossip::GossipType,
    lan::playerdata,
    mobs::{Mob, MobEvent, Species, drops, finalize, villager, zombie_villager},
};
use sunrise_world::food::Difficulty;

/// The `Zombie` subclasses that convert the villagers they kill.
fn is_zombie(s: Species) -> bool {
    matches!(
        s,
        Species::Zombie
            | Species::Husk
            | Species::Drowned
            | Species::ZombieVillager
            | Species::ZombifiedPiglin
    )
}
/// Whether a zombie turns the villager it killed: always on Hard, half the
/// time on Normal (`Zombie.killedEntity`).
fn infects(difficulty: Difficulty, coin: impl FnOnce() -> bool) -> bool {
    match difficulty {
        Difficulty::Hard => true,
        Difficulty::Normal => !coin(),
        _ => false,
    }
}

impl crate::game::ServerGame<'_> {
    /// `Zombie.killedEntity`: a villager killed by a zombie may rise as a
    /// zombie villager keeping its profession, trades and gossip.
    pub(in crate::game) fn zombie_killed(&mut self, killer: u32, victim: u32) {
        let Some(k) = self.common.mobs.iter().position(|m| m.id == killer) else {
            return;
        };
        let Some(v) = self.common.mobs.iter().position(|m| m.id == victim) else {
            return;
        };
        if !is_zombie(self.common.mobs[k].species())
            || self.common.mobs[v].species() != Species::Villager
            || self.common.mobs[v].alive()
            || self.common.mobs[v].removed
        {
            return;
        }
        let difficulty = self.common.world.health_rules.difficulty;
        let random = &mut self.common.mobs[k].random;
        if !infects(difficulty, || random.next_boolean()) {
            return;
        }
        // The villager is gone without dying: no drops, no death animation.
        let old = self.common.mobs[v].clone();
        self.common.mobs[v].removed = true;
        self.common.mobs[v].health = 0.0;
        let at = [old.position.x, old.position.y, old.position.z];
        let Some(id) = self.spawn_mob(
            "minecraft:zombie_villager",
            at,
            old.persistent,
            sunrise_gameplay_common::game::creatures::SpawnSite::Finalizing(
                sunrise_sim::mobs::spawn_rules::Reason::Conversion,
            ),
        ) else {
            return;
        };
        let ensure = self.common.mobs.iter().position(|m| m.id == victim);
        if let Some(i) = ensure {
            self.ensure_offers(i);
        }
        let old = ensure.map_or(old, |i| self.common.mobs[i].clone());
        let Some(z) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        convert_to_zombie_villager(&old, z);
        for rider in self.common.mobs.iter_mut() {
            if rider.extra.mount == Some(old.id) {
                rider.extra.mount = Some(id);
            }
        }
        if old.extra.mount.is_some() {
            self.position_riders();
        }
        if !self.common.mobs[k].silent {
            let at = self.common.mobs[k].position;
            self.server_level_event_at(
                1026,
                [
                    at.x.floor() as i32,
                    at.y.floor() as i32,
                    at.z.floor() as i32,
                ],
                0,
            );
        }
    }

    /// `ZombieVillager.mobInteract`: a golden apple on a weakened zombie
    /// villager starts the cure and is eaten; without weakness it does
    /// nothing. Returns whether the click was used.
    pub(in crate::game) fn interact_zombie_villager(&mut self, index: usize, item: &str) -> bool {
        let m = &self.common.mobs[index];
        if m.species() != Species::ZombieVillager || item != "minecraft:golden_apple" {
            return false;
        }
        if !m.has_effect("minecraft:weakness") {
            return true;
        }
        self.consume_held();
        // `player.getUUID()`: the acting player, a server's guest included.
        let starter = Some(self.acting_player_uuid());
        let time = zombie_villager::WAIT_MIN
            + self.common.mobs[index]
                .random
                .next_int_bound(zombie_villager::WAIT_SPREAD)
                .unwrap_or(0);
        let id = self.common.mobs[index].id;
        let at = [
            self.common.mobs[index].position.x,
            self.common.mobs[index].position.y,
            self.common.mobs[index].position.z,
        ];
        villager::ext_mut(&mut self.common.mobs[index]).cure =
            Some(zombie_villager::Cure { time, starter });
        if let Some(def) = sunrise_game_data::potions::effect("weakness") {
            self.remove_mob_effect_by_index(id, def.index);
        }
        self.add_mob_effect(id, EffectInstance::new("strength", time, 0), None);
        let _ = self.handle_mob_events(vec![MobEvent::EntityEvent {
            mob: id,
            event: 16,
            at,
        }]);
        self.swing_arm();
        true
    }

    /// `ZombieVillager.finishConversion`: the villager returns with its
    /// data, trades, experience and gossip, and the curing player earns the
    /// village's trust.
    pub(in crate::game) fn cure_zombie_villager(&mut self, id: u32) {
        let Some(i) = self.common.mobs.iter().position(|m| m.id == id) else {
            return;
        };
        if self.common.mobs[i].removed || zombie_villager::cure(&self.common.mobs[i]).is_none() {
            return;
        }
        let old = self.common.mobs[i].clone();
        self.common.mobs[i].removed = true;
        let at = [old.position.x, old.position.y, old.position.z];
        let Some(vid) = self.spawn_mob(
            "minecraft:villager",
            at,
            old.persistent,
            sunrise_gameplay_common::game::creatures::SpawnSite::Finalizing(
                sunrise_sim::mobs::spawn_rules::Reason::Conversion,
            ),
        ) else {
            return;
        };
        // The villager's `addEffect(CONFUSION, 200)` asks `canBeAffected`
        // first, as in vanilla, on the fresh villager.
        let nausea =
            self.can_be_affected(Some(vid as i32), "minecraft:villager", "minecraft:nausea");
        let Some(v) = self.common.mobs.iter_mut().find(|m| m.id == vid) else {
            return;
        };
        let drops = convert_to_villager(&old, v, nausea);
        for rider in self.common.mobs.iter_mut() {
            if rider.extra.mount == Some(old.id) {
                rider.extra.mount = Some(vid);
            }
        }
        if old.extra.mount.is_some() {
            self.position_riders();
        }
        for stack in drops {
            self.spawn_item(
                stack,
                [old.position.x, old.position.y, old.position.z],
                [0.0; 3],
                10,
            );
        }
        // `level.getPlayerByUUID(conversionStarter)`: the player who fed the
        // apple, if still in this level (the host or a server's guest).
        let starter = zombie_villager::cure(&old).and_then(|c| c.starter);
        let curer = starter.and_then(|uuid| Some((uuid, self.player_here(uuid)?)));
        // `Villager.onReputationEventFrom(ZOMBIE_VILLAGER_CURED)`.
        let Some(v) = self.common.mobs.iter_mut().find(|m| m.id == vid) else {
            return;
        };
        if let Some((uuid, _)) = curer {
            let g = &mut villager::ext_mut(v).gossips;
            g.add(uuid, GossipType::MajorPositive, 20);
            g.add(uuid, GossipType::MinorPositive, 25);
        }
        let cured = v.clone();
        let villager_view = self.mob_entity(&cured);
        let zombie_view = self.mob_entity(&old);
        if let Some((_, who)) = curer {
            self.for_player(who, |g| {
                g.trigger_cured_zombie_villager(&zombie_view, &villager_view)
            });
        }
        // The nausea comes after the advancement criterion (finishConversion).
        self.add_mob_effect(vid, EffectInstance::new("nausea", 200, 0), None);
        if !old.silent {
            self.server_level_event_at(
                1027,
                [
                    old.position.x.floor() as i32,
                    old.position.y.floor() as i32,
                    old.position.z.floor() as i32,
                ],
                0,
            );
        }
    }

    /// `ServerLevel.getPlayerByUUID`: the player with `uuid` in this level.
    fn player_here(&self, uuid: [i32; 4]) -> Option<Target> {
        match self.target_by_uuid(playerdata::ints_uuid(uuid))? {
            Target::Player => Some(Target::Player),
            Target::Guest(conn) => {
                let g = self.server.lan.as_ref()?.hub.guest(conn)?;
                (g.dimension == self.common.dim.id()).then_some(Target::Guest(conn))
            }
            _ => None,
        }
    }

    /// `CuredZombieVillagerTrigger`.
    fn trigger_cured_zombie_villager(
        &mut self,
        zombie: &adv::EntityView,
        villager: &adv::EntityView,
    ) {
        let me = self.player_entity_for("minecraft:cured_zombie_villager");
        let (zombie, villager) = (zombie.clone(), villager.clone());
        self.trigger("minecraft:cured_zombie_villager", move |c, w| {
            let origin = me.position;
            let ctx = |e| adv::Entities {
                this: Some(e),
                origin,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            adv::conditions(&c["player"], &ctx(&me), w)
                && adv::conditions(&c["zombie"], &ctx(&zombie), w)
                && adv::conditions(&c["villager"], &ctx(&villager), w)
        });
    }
}

/// `Mob.convertTo` (`ConversionType.SINGLE`, keeping equipment and the
/// pick-up-loot flag): what any converted mob carries over.
fn carry_over(from: &Mob, to: &mut Mob) {
    to.position = from.position;
    to.previous = from.position;
    to.velocity = from.velocity;
    to.yaw = from.yaw;
    to.previous_yaw = from.yaw;
    to.body_yaw = from.body_yaw;
    to.previous_body_yaw = from.body_yaw;
    to.head_yaw = from.head_yaw;
    to.previous_head_yaw = from.head_yaw;
    to.head_pitch = from.head_pitch;
    to.previous_head_pitch = from.head_pitch;
    to.fall_distance = from.fall_distance;
    to.hurt_time = from.hurt_time;
    to.ground.on_ground = from.ground.on_ground;
    to.fire_ticks = from.fire_ticks;
    to.persistent = from.persistent;
    to.custom_name = from.custom_name.clone();
    to.custom_name_text = from.custom_name_text.clone();
    to.name_visible = from.name_visible;
    to.invisible = from.invisible;
    to.extra.pick_up_loot = from.extra.pick_up_loot;
    // ConversionType.convertCommon: the flags a converted mob keeps.
    to.extra.left_handed = from.extra.left_handed;
    to.no_ai = from.no_ai;
    to.invulnerable_flag = from.invulnerable_flag;
    to.invulnerable_time = from.invulnerable_time;
    to.no_gravity = from.no_gravity;
    to.silent = from.silent;
    to.extra.mount = from.extra.mount;
    to.extra.passenger = from.extra.passenger;
    to.fluids.boat = from.fluids.boat;
    to.extra.leash = from.extra.leash;
    to.extra.leash_pending = from.extra.leash_pending;
    to.extra.leash_player = from.extra.leash_player;
    for slot in sunrise_items::equipment::MAINHAND..=sunrise_items::equipment::BODY {
        to.extra
            .set_slot_stack(slot, from.extra.slot_stack(slot).cloned());
    }
    to.extra.gear = from.extra.gear.clone();
    to.retain_effects(|_| false);
    // `convertCommon` copies each effect with `addEffect`: one the new mob
    // cannot be affected by is refused (an undead zombie villager takes no
    // Poison or Regeneration), and `onEffectAdded` stores the attribute
    // modifiers of the rest. `new MobEffectInstance(effect)` keeps the icon flag.
    for effect in &from.extra.effects {
        to.add_effect(effect.id, effect.duration, effect.amplifier);
        if let Some(copy) = to.extra.effects.iter_mut().find(|e| e.id == effect.id) {
            copy.show_icon = effect.show_icon;
        }
    }

    // VillagerData and the trades, experience and gossip.
    to.extra.variant = from.extra.variant;
    to.extra.profession = from.extra.profession.clone();
    to.extra.villager_level = from.extra.villager_level.max(1);
    to.extra.trade = from.extra.trade.clone();
    if let Some(g) = villager::ext(from).map(|e| e.gossips.clone()) {
        villager::ext_mut(to).gossips = g;
    }
}
/// A villager rises as a zombie villager (never as an adult-age baby: a baby
/// stays a baby for good).
pub(in crate::game::server_side) fn convert_to_zombie_villager(from: &Mob, to: &mut Mob) {
    carry_over(from, to);
    to.extra.age = if from.is_baby() { i32::MIN / 2 } else { 0 };
}
/// A cured zombie villager becomes a villager again, queasy for 10 seconds
/// when `nausea` (the `canBeAffected` ask on the fresh villager) allows it.
pub fn convert_to_villager(from: &Mob, to: &mut Mob, nausea: bool) -> Vec<ItemStack> {
    carry_over(from, to);
    let mut drops_out = Vec::new();
    // `EquipmentSlot.VALUES` order: main hand, off hand, feet, legs, chest, head.
    for (slot, name) in drops::SLOTS.iter().take(6).enumerate() {
        let Some(stack) = finalize::item_stack(from, slot) else {
            continue;
        };
        drops::clear_slot(to, name);
        if stack.enchantment_level("minecraft:binding_curse") > 0 {
            // `getIndex() + 300`: hands 0..1, then feet, legs, chest, head 0..3.
            let index = if slot < 2 { slot } else { slot - 2 };
            let inventory = &mut villager::ext_mut(to).inventory;
            if index < inventory.len() {
                inventory[index] = stack;
            } else {
                inventory.push(stack);
            }
        } else if drops::drop_chance(from, name) > 1.0 {
            drops_out.push(stack);
        }
    }
    to.extra.age = if from.is_baby() { -24000 } else { 0 };
    if nausea {
        to.add_effect("minecraft:nausea", 200, 0);
    }
    drops_out
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

use sunrise_gameplay_common::game::passengers_common::PassengersCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn conversions_preserve_body_equipment() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        let types = sunrise_sim::mobs::MobTypes::load(&root).unwrap();
        let mob = |id, name: &str| {
            Mob::new(
                id,
                std::sync::Arc::new(types.types[name].clone()),
                sunrise_core::geometry::Vec3::ZERO,
                0.0,
            )
        };
        let mut source = mob(1, "minecraft:villager");
        let mut armor = sunrise_items::items::ItemStack::new("minecraft:wolf_armor", 1);
        armor.damage = 7;
        armor.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("kept".into()));
        source.extra.set_body_stack(Some(armor));
        let mut zombie = mob(2, "minecraft:zombie_villager");
        convert_to_zombie_villager(&source, &mut zombie);
        assert_eq!(zombie.extra.body_id(), Some("minecraft:wolf_armor"));
        assert_eq!(zombie.extra.body_damage(), 7);
        assert_eq!(zombie.extra.body_stack(), source.extra.body_stack());
        let mut cured = mob(3, "minecraft:villager");
        convert_to_villager(&zombie, &mut cured, true);
        assert_eq!(cured.extra.body_id(), Some("minecraft:wolf_armor"));
        assert_eq!(cured.extra.body_damage(), 7);
        assert_eq!(cured.extra.body_stack(), source.extra.body_stack());
    }

    #[test]
    fn zombies_convert_by_difficulty() {
        assert!(infects(Difficulty::Hard, || unreachable!()));
        assert!(!infects(Difficulty::Easy, || false));
        assert!(!infects(Difficulty::Peaceful, || false));
        // Normal: a coin toss that spares the villager when it is true.
        assert!(infects(Difficulty::Normal, || false));
        assert!(!infects(Difficulty::Normal, || true));
    }
    #[test]
    fn every_zombie_kind_infects() {
        for s in [
            Species::Zombie,
            Species::Husk,
            Species::Drowned,
            Species::ZombieVillager,
            Species::ZombifiedPiglin,
        ] {
            assert!(is_zombie(s));
        }
        assert!(!is_zombie(Species::Skeleton) && !is_zombie(Species::Villager));
    }

    /// A villager-shaped mob of `species` with the registered `attributes`.
    fn test_mob(id: u32, species: Species, name: &str, attributes: &[(&str, f64)]) -> Mob {
        use std::sync::Arc;
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::{Anim, Brain, MobType};
        let kind = MobType {
            type_key: Default::default(),
            id: name.into(),
            species,
            width: 0.6,
            height: 1.95,
            eye_height: 1.74,
            category: "monster".into(),
            max_health: 20.0,
            speed: 0.23,
            flying_speed: 0.4,
            attack_damage: 3.0,
            attack_knockback: 0.0,
            follow_range: 35.0,
            step_height: 0.6,
            armor: 0.0,
            armor_toughness: 0.0,
            knockback_resistance: 0.0,
            jump_strength: 0.42,
            attributes: Arc::new(
                attributes
                    .iter()
                    .map(|(name, v)| ((*name).to_owned(), *v))
                    .collect(),
            ),
            item_stack_limits: Arc::default(),
            attribute_bounds: Default::default(),
            gravity: 0.08,
            safe_fall_distance: 3.0,
            fall_damage_multiplier: 1.0,
            water_efficiency: 0.0,
            water_slowdown: 0.8,
            air_speed: 0.02,
            baby_scale: 0.5,
            head_rot_speed: 10.0,
            max_head_x_rot: 40.0,
            max_head_y_rot: 75.0,
            ambient_interval: 80,
            flags: Default::default(),
            brain: Brain::Passive,
            anim: Anim::Humanoid { zombie_arms: true },
            sounds: ["entity.villager.ambient"; 3],
            sound_overrides: Default::default(),
            food: Arc::default(),
            tempt: Arc::default(),
            passenger_attachments: Vec::new(),
        };
        Mob::new(id, Arc::new(kind), Vec3::ZERO, 0.0)
    }

    /// #478: `convertCommon` copies each effect with `addEffect`, so the
    /// cured villager and the infected zombie villager hold each effect's
    /// attribute modifier (0.2F x 2 for Speed II), once.
    #[test]
    fn a_conversion_stores_the_copied_effects_modifiers() {
        use sunrise_sim::mobs::finalize::{Attr, modifier_amount, value};
        let speed = [("minecraft:movement_speed", 0.5)];
        let mut villager = test_mob(1, Species::Villager, "minecraft:villager", &speed);
        villager.add_effect("minecraft:speed", -1, 1);
        let mut zombie = test_mob(
            2,
            Species::ZombieVillager,
            "minecraft:zombie_villager",
            &speed,
        );
        convert_to_zombie_villager(&villager, &mut zombie);
        assert_eq!(
            modifier_amount(&zombie, Attr::MovementSpeed, "minecraft:effect.speed"),
            Some(0.400_000_005_960_464_5)
        );
        assert_eq!(value(&zombie, Attr::MovementSpeed), 0.700_000_002_980_232_2);
        let mut cured = test_mob(3, Species::Villager, "minecraft:villager", &speed);
        convert_to_villager(&zombie, &mut cured, false);
        assert_eq!(
            modifier_amount(&cured, Attr::MovementSpeed, "minecraft:effect.speed"),
            Some(0.400_000_005_960_464_5)
        );
        let held = cured.extra.attributes.as_ref().unwrap();
        assert_eq!(
            held.modifiers
                .iter()
                .filter(|x| x.id == "minecraft:effect.speed")
                .count(),
            1
        );
    }

    /// #478: the copies go through `addEffect`, so a poisoned, regenerating
    /// villager's infection leaves the undead zombie villager with neither
    /// (`canBeAffected`), while its Speed comes along with the modifier and
    /// the icon flag it had.
    #[test]
    fn an_infected_villager_keeps_only_the_effects_a_zombie_villager_takes() {
        use sunrise_sim::mobs::finalize::{Attr, modifier_amount};
        let speed = [("minecraft:movement_speed", 0.5)];
        let mut villager = test_mob(1, Species::Villager, "minecraft:villager", &speed);
        villager.add_effect("minecraft:poison", 200, 0);
        villager.add_effect("minecraft:regeneration", 200, 0);
        villager.add_effect("minecraft:speed", 200, 0);
        villager.extra.effects[2].show_icon = false;
        let mut zombie = test_mob(
            2,
            Species::ZombieVillager,
            "minecraft:zombie_villager",
            &speed,
        );
        std::sync::Arc::make_mut(&mut zombie.kind).flags.undead = true;
        convert_to_zombie_villager(&villager, &mut zombie);
        let held: Vec<_> = zombie
            .extra
            .effects
            .iter()
            .map(|e| (e.id, e.duration, e.amplifier, e.show_icon))
            .collect();
        assert_eq!(held, [("minecraft:speed", 200, 0, false)]);
        assert_eq!(
            modifier_amount(&zombie, Attr::MovementSpeed, "minecraft:effect.speed"),
            Some(0.200_000_002_980_232_24)
        );
    }

    /// `ConversionType.convertCommon` copies Silent, NoAI, LeftHanded,
    /// NoGravity and Invulnerable onto the mob that replaces the old one.
    #[test]
    fn infection_and_the_cure_keep_the_common_flags() {
        let mob = |id, species, name: &str| test_mob(id, species, name, &[]);
        let mut villager = mob(1, Species::Villager, "minecraft:villager");
        villager.silent = true;
        villager.no_ai = true;
        villager.no_gravity = true;
        villager.invulnerable_flag = true;
        villager.extra.left_handed = true;
        villager.extra.mount = Some(99);
        villager.extra.passenger = true;
        let mut zombie = mob(2, Species::ZombieVillager, "minecraft:zombie_villager");
        assert!(!zombie.silent && !zombie.no_ai && !zombie.extra.left_handed);
        assert_eq!(zombie.extra.mount, None);
        assert!(!zombie.extra.passenger);
        convert_to_zombie_villager(&villager, &mut zombie);
        assert!(zombie.silent);
        assert!(zombie.no_ai);
        assert!(zombie.no_gravity);
        assert!(zombie.invulnerable_flag);
        assert!(zombie.extra.left_handed);
        assert_eq!(zombie.extra.mount, Some(99));
        assert!(zombie.extra.passenger);
        let mut cured = mob(3, Species::Villager, "minecraft:villager");
        convert_to_villager(&zombie, &mut cured, true);
        assert!(cured.silent);
        assert!(cured.no_ai);
        assert!(cured.no_gravity);
        assert!(cured.invulnerable_flag);
        assert!(cured.extra.left_handed);
        assert_eq!(cured.extra.mount, Some(99));
        assert!(cured.extra.passenger);
        // The cure keeps no equipment: picked-up gear drops, cursed gear goes
        // to the inventory, the rest vanishes.
        use sunrise_sim::mobs::finalize::{
            FEET, HEAD, MAINHAND, item_stack, set_drop_chance, set_item, set_item_stack,
        };
        let mut gear = mob(4, Species::ZombieVillager, "minecraft:zombie_villager");
        set_item(&mut gear, MAINHAND, "minecraft:iron_sword");
        set_drop_chance(&mut gear, MAINHAND, 2.0);
        set_item(&mut gear, HEAD, "minecraft:iron_helmet");
        let mut boots = ItemStack::new("minecraft:iron_boots", 1);
        boots.set_crafting_enchantments(vec![("minecraft:binding_curse".into(), 1)]);
        set_item_stack(&mut gear, FEET, boots);
        let mut bare = mob(5, Species::Villager, "minecraft:villager");
        let drops = convert_to_villager(&gear, &mut bare, true);
        assert_eq!(drops.len(), 1);
        assert_eq!(drops[0].item.as_ref(), "minecraft:iron_sword");
        for slot in 0..=5 {
            assert!(item_stack(&bare, slot).is_none(), "slot {slot}");
        }
        let kept = &villager::ext(&bare).unwrap().inventory;
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].item.as_ref(), "minecraft:iron_boots");
    }
}
