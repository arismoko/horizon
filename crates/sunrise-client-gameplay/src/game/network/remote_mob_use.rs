//! A server's client right-clicking a mob: vanilla's client runs
//! `Player.interactOn` on its own level (`MultiPlayerGameMode.interact`) to
//! learn whether the click ends on the mob or goes on to the held item
//! (`Minecraft.startUseItem` then sends UseItem). The server does the real
//! interaction; this is what the client's copy of the mob answers.
use crate::game::ClientGame;
use sunrise_client::actions::Action;
use sunrise_client_gameplay::remote::mob_use::{
    ages_on_food, copper_golem_takes, iron_golem_takes, mob_swings, nautilus_takes,
};
use sunrise_gameplay_common::game::item_abilities::{ItemAbilitiesGameExt1 as _, ability};
use sunrise_items::items::ItemStack;
use sunrise_sim::first_person_sim::Hand;
use sunrise_sim::mobs::{Mob, Species, armadillo, equine, llama, nautilus, panda, villager};

/// `Mob.checkAndHandleImportantInteractions`: a lead, a named name tag, a spawn egg of the
/// mob's own kind.
fn important_interaction(m: &Mob, stack: Option<&ItemStack>) -> bool {
    let item = stack.map_or("", |s| &*s.item);
    item == "minecraft:lead"
        || item == "minecraft:name_tag" && stack.is_some_and(|s| s.custom_name().is_some())
        || item.strip_suffix("_spawn_egg") == Some(m.kind.id.as_str())
}

impl ClientGame<'_> {
    /// Whether mob `i` takes a right click with `hand`'s item on this
    /// client (`Mob.interact`'s result consumes the action).
    pub fn remote_mob_takes(&mut self, i: usize, hand: Hand) -> bool {
        self.remote_mob_answers(i, hand, true)
    }

    /// The same answer, changing nothing: an allay's and a dyed sheep's answer is the
    /// interaction itself, and the controller's guide asks every frame.
    pub fn remote_mob_would_take(&mut self, i: usize, hand: Hand) -> bool {
        self.remote_mob_answers(i, hand, false)
    }

    fn remote_mob_answers(&mut self, i: usize, hand: Hand, act: bool) -> bool {
        let Some(m) = self.common.mobs.get(i).filter(|m| m.alive()) else {
            return false;
        };
        let stack = match hand {
            Hand::Main => self.common.stack_in(self.common.selected),
            Hand::Off => self.common.offhand.as_ref(),
        };
        let item = stack.map_or("", |s| &*s.item);
        let shears = stack.is_some_and(|s| self.stack_shears(s, ability::SHEARS_HARVEST));
        let baby = m.is_baby();
        if important_interaction(m, stack) {
            return true;
        }
        // Armadillo.mobInteract: a rolled-up armadillo fails before
        // AgeableMob/Animal.mobInteract, so food and age lock are not taken.
        // A brush still reaches brushOffScute below.
        if m.species() == Species::Armadillo && armadillo::client_rejects_food(m, item) {
            return false;
        }
        // Panda.mobInteract: a scared panda passes and one lying on its back
        // takes any item (it gets up on the server). Its bamboo is taken on a
        // client only by a baby that ages up; an adult's breeding and eating
        // need a ServerLevel, so the click passes to the item.
        if m.species() == Species::Panda {
            if panda::scared(m) {
                return false;
            }
            if panda::is_on_back(m) {
                return true;
            }
            if m.kind.food.iter().any(|f| f.as_str() == item) {
                return baby && !m.extra.age_locked;
            }
        }
        // AgeableMob.mobInteract: a golden dandelion on a baby that can be age
        // locked is SUCCESS on a client too (the server sets the lock).
        if self.age_lock_takes(m, item) {
            return true;
        }
        // Animal.mobInteract: its food is CONSUME on a client. A nautilus
        // overrides mobInteract, and an untamed one only takes its taming
        // items, so it answers in its own arm below.
        if m.kind.flags.animal
            && !matches!(m.species(), Species::Nautilus | Species::ZombieNautilus)
            && m.kind.food.iter().any(|f| f.as_str() == item)
        {
            return true;
        }
        let sneaking = self.held_mapping(Action::Sneak);
        match m.species() {
            // Villager.mobInteract: trading, or a baby's head shake. A sleeping
            // villager falls back to super.mobInteract, which passes.
            Species::Villager => item != "minecraft:villager_spawn_egg" && !villager::sleeping(m),
            // WanderingTrader.mobInteract: trading.
            Species::WanderingTrader => item != "minecraft:villager_spawn_egg",
            // AbstractHorse, Camel: ridden (or their inventory opened); a baby only eats.
            s if equine::is_equine(s)
                || llama::is_llama(s)
                || matches!(s, Species::Camel | Species::CamelHusk) =>
            {
                !baby
            }
            // Pig / Strider.mobInteract: a saddled one nobody rides is ridden
            // (`!isVehicle()`).
            Species::Pig | Species::Strider => {
                m.extra.saddled()
                    && !sneaking
                    && self.common.riding != Some(m.id)
                    && !self.common.mobs.iter().any(|o| o.extra.mount == Some(m.id))
            }
            // HappyGhast.mobInteract: a harnessed adult is ridden.
            Species::HappyGhast => !baby && m.extra.body_id().is_some(),
            // TamableAnimal (wolf, cat, parrot) on a client: CONSUME once tame.
            Species::Wolf | Species::Cat | Species::Parrot => m.extra.tame,
            // Allay.mobInteract: gives an item or takes it back.
            Species::Allay if !act => self.allay_would_take(i, hand, stack.cloned()),
            Species::Allay => self.predict_allay_interaction(i, hand, stack.cloned()),
            Species::Cow | Species::Goat => item == "minecraft:bucket" && !baby,
            Species::Mooshroom => {
                !baby
                    && (matches!(item, "minecraft:bucket" | "minecraft:bowl")
                        || shears
                        || m.extra.variant == 1
                            && self.items().suspicious_stew_effects(item).is_some())
            }
            Species::Sheep if !act => shears || self.sheep_would_take_dye(i, hand),
            Species::Sheep => shears || self.client_interact_sheep_dye(i, hand),
            // SnowGolem.mobInteract: shears take the pumpkin off.
            Species::SnowGolem => shears && !m.extra.sheared,
            // AbstractNautilus.mobInteract: an untamed adult only answers its
            // taming items, a tame one is fed, mounted or opens its screen.
            Species::Nautilus | Species::ZombieNautilus => {
                let taming: Vec<String> = self
                    .common
                    .res
                    .items()
                    .item_tags
                    .get("minecraft:nautilus_taming_items")
                    .map(|t| t.iter().cloned().collect())
                    .unwrap_or_default();
                nautilus_takes(nautilus::is_food(m, item, &taming), m.extra.tame, baby)
            }
            // Dolphin.mobInteract: a #minecraft:fishes item is SUCCESS on both
            // sides (the server feeds it), so the client swings and keeps the fish.
            Species::Dolphin => self
                .common
                .res
                .items()
                .item_tags
                .get("minecraft:fishes")
                .is_some_and(|t| t.contains(item)),
            // IronGolem.mobInteract.
            Species::IronGolem => iron_golem_takes(item, m.health, m.attrs.max_health),
            // CopperGolem.mobInteract.
            Species::CopperGolem => copper_golem_takes(
                item,
                shears,
                m.extra.main_hand_stack().is_some(),
                self.common.golem_shearable(i),
            ),
            // Bucketable.bucketMobPickup.
            Species::Cod
            | Species::Salmon
            | Species::TropicalFish
            | Species::Pufferfish
            | Species::Axolotl => item == "minecraft:water_bucket",
            // SulfurCube.mobInteract ends in Bucketable.bucketMobPickup, which
            // also runs on a client: an empty bucket on an unprimed adult
            // (its swallowable items and shears are answered before that).
            Species::SulfurCube => {
                item == "minecraft:bucket"
                    && !baby
                    && !sunrise_sim::mobs::sulfur_cube::primed(m)
                    && !sunrise_sim::mobs::sulfur_cube::swallowable(m, item)
            }
            // Tadpole.mobInteract: frog food (not while age locked), else the bucket.
            Species::Tadpole => {
                item == "minecraft:water_bucket"
                    || !m.extra.age_locked && m.kind.food.iter().any(|f| f.as_str() == item)
            }
            // Armadillo.brushOffScute fails on a baby, so its brush falls
            // through to FAIL (scared) or Mob.mobInteract's PASS.
            Species::Armadillo => {
                !baby
                    && stack.is_some_and(|s| {
                        self.stack_can_perform(s, ability::BRUSH_BRUSH, item == "minecraft:brush")
                    })
            }
            // ZombieVillager.mobInteract: CONSUME for a golden apple.
            Species::ZombieVillager => item == "minecraft:golden_apple",
            // Creeper.mobInteract: CREEPER_IGNITERS.
            Species::Creeper => {
                matches!(item, "minecraft:flint_and_steel" | "minecraft:fire_charge")
            }
            // Piglin.mobInteract on a client: PiglinAi.canAdmire (an adult and
            // barter currency; the client's brain holds no admiring memories),
            // unless getArmPose is ADMIRING_ITEM (a loved off-hand item, and
            // not dancing).
            Species::Piglin => {
                !baby
                    && stack.is_some_and(|s| self.piglin_currency(s))
                    && (m.extra.scratch[0] > 0
                        || !m.extra.off_hand_id().is_some_and(|o| self.piglin_loved(o)))
            }
            _ => false,
        }
    }
    /// Whether that answer is `InteractionResult.SUCCESS`, the only result that
    /// swings the hand: `CONSUME` ends the click without one. An adult animal's
    /// food (`Animal.mobInteract`), a zombie villager's golden apple and a
    /// sheep's shears (the server alone checks `readyForShearing`) answer
    /// CONSUME on a client, and a horse's or panda's food SUCCESS_SERVER
    /// (#9437). A tame wolf, cat or parrot sitting down is SUCCESS.
    pub(in crate::game::client_side) fn remote_mob_swings(&self, i: usize, hand: Hand) -> bool {
        let Some(m) = self.common.mobs.get(i).filter(|m| m.alive()) else {
            return false;
        };
        let stack = match hand {
            Hand::Main => self.common.stack_in(self.common.selected),
            Hand::Off => self.common.offhand.as_ref(),
        };
        let item = stack.map_or("", |s| &*s.item);
        // NameTagItem.interactLivingEntity: only a named tag answers (SUCCESS);
        // an unnamed one passes on to the trader's trading (SUCCESS_SERVER).
        if m.species() == Species::WanderingTrader && item == "minecraft:name_tag" {
            return stack.is_some_and(|s| s.custom_name().is_some());
        }
        // MushroomCow.mobInteract: a stew source answers SUCCESS before ordinary food.
        if m.species() == Species::Mooshroom
            && m.extra.variant == 1
            && !m.is_baby()
            && self.items().suspicious_stew_effects(item).is_some()
        {
            return true;
        }
        // Sheep.mobInteract (after the important interactions): shears, sheared or not,
        // answer CONSUME on a client.
        if m.species() == Species::Sheep
            && !important_interaction(m, stack)
            && stack.is_some_and(|s| self.stack_shears(s, ability::SHEARS_HARVEST))
        {
            return false;
        }
        mob_swings(m, item)
    }
    /// `AbstractHorse.handleEating` (`Llama`'s and `Camel`'s too) on this
    /// client's copy: a baby fed something that ages it draws one happy
    /// villager particle. The server draws it on its own level only, so no
    /// packet brings it.
    pub(in crate::game::client_side) fn remote_feed_particles(&mut self, i: usize, hand: Hand) {
        let Some(m) = self.common.mobs.get(i).filter(|m| m.alive()) else {
            return;
        };
        let stack = match hand {
            Hand::Main => self.common.stack_in(self.common.selected),
            Hand::Off => self.common.offhand.as_ref(),
        };
        let item = stack.map_or("", |s| &*s.item);
        if !ages_on_food(m, item) {
            return;
        }
        let feet = [m.position.x, m.position.y, m.position.z];
        let (width, height) = (f64::from(m.width()), f64::from(m.height()));
        let options = sunrise_level::particle_options::ParticleOptions::simple(
            sunrise_level::particle_options::ParticleType::HappyVillager,
        );
        let mut spawned = Vec::new();
        sunrise_level::particle_options::around_self(
            self.client.particles.0.random(),
            feet,
            width,
            height,
            sunrise_level::particle_options::Burst::HORSE_EATING,
            |p, v| spawned.push((p, v)),
        );
        for (p, v) in spawned {
            self.client.particles.0.add(&options, p, v);
        }
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::piglins_common::PiglinsCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_sim::mobs::Mob;

    fn mob(kind: &str) -> Option<Mob> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return None;
        }
        let data = sunrise_sim::mobs::MobTypes::load(&root).ok()?;
        let kind = Arc::new(data.types.get(kind)?.clone());
        Some(Mob::new(
            1,
            kind,
            sunrise_core::geometry::Vec3::new(0.0, 4.0, 0.0),
            0.0,
        ))
    }

    /// Tamables' and horses' own `mobInteract` answer before `Animal.mobInteract`'s
    /// CONSUME (#9450): sitting down and healing are SUCCESS, a horse's food
    /// SUCCESS_SERVER.
    #[test]
    fn only_a_client_success_swings_the_hand() {
        let Some(mut wolf) = mob("minecraft:wolf") else {
            return;
        };
        wolf.extra.tame = true;
        assert!(
            mob_swings(&wolf, ""),
            "a tame wolf sits: SUCCESS.withoutItem"
        );
        assert!(
            !mob_swings(&wolf, "minecraft:beef"),
            "full health: Animal's CONSUME"
        );
        wolf.health = wolf.attrs.max_health - 1.0;
        assert!(
            mob_swings(&wolf, "minecraft:beef"),
            "hurt: Wolf.feed is SUCCESS"
        );
        let mut cat = mob("minecraft:cat").unwrap();
        assert!(mob_swings(&cat, "minecraft:cod"), "taming: SUCCESS");
        cat.extra.tame = true;
        assert!(
            !mob_swings(&cat, "minecraft:cod"),
            "tame, full health: CONSUME"
        );
        assert!(mob_swings(&cat, ""), "a tame cat sits: SUCCESS");
        let mut parrot = mob("minecraft:parrot").unwrap();
        parrot.extra.tame = true;
        assert!(mob_swings(&parrot, ""), "a tame parrot sits: SUCCESS");
        let horse = mob("minecraft:horse").unwrap();
        assert!(
            !mob_swings(&horse, "minecraft:apple"),
            "AbstractHorse.fedFood: SUCCESS_SERVER"
        );
        let cow = mob("minecraft:cow").unwrap();
        assert!(!mob_swings(&cow, "minecraft:wheat"), "adult: CONSUME");
        let sheep = mob("minecraft:sheep").unwrap();
        assert!(!mob_swings(&sheep, "minecraft:shears"), "Sheep: CONSUME");
    }

    /// `AbstractNautilus.mobInteract` (#8595): an untamed adult takes its taming
    /// items only, and a tame adult is mounted by any click.
    #[test]
    fn a_nautilus_answers_its_food_and_a_tame_one_is_ridden() {
        assert!(
            !nautilus_takes(false, false, false),
            "untamed, taming item missing"
        );
        assert!(nautilus_takes(true, false, false), "a taming item");
        assert!(
            nautilus_takes(false, true, false),
            "a tame adult is mounted"
        );
        assert!(!nautilus_takes(false, true, true), "a baby is not mounted");

        // `AbstractNautilus.isFood`: taming items while untamed, its food after.
        let Some(mut m) = mob("minecraft:nautilus") else {
            return;
        };
        let taming = vec!["minecraft:pufferfish".to_string()];
        m.extra.age = 0;
        m.extra.tame = false;
        assert!(nautilus::is_food(&m, "minecraft:pufferfish", &taming));
        assert!(
            !nautilus::is_food(&m, "minecraft:cod", &taming),
            "cod tames nothing"
        );
        m.extra.tame = true;
        assert!(
            nautilus::is_food(&m, "minecraft:cod", &taming),
            "a tame one eats cod"
        );
    }

    /// `IronGolem.mobInteract` and `CopperGolem.mobInteract` (#8630): the click
    /// has to end on the golem, or the client goes on to use the item and turns
    /// to the off hand.
    #[test]
    fn the_golems_answer_the_click() {
        assert!(iron_golem_takes("minecraft:iron_ingot", 50.0, 100.0));
        assert!(
            !iron_golem_takes("minecraft:iron_ingot", 100.0, 100.0),
            "full"
        );
        assert!(
            !iron_golem_takes("minecraft:stick", 50.0, 100.0),
            "not ingot"
        );

        assert!(
            copper_golem_takes("", false, true, false),
            "takes its item back"
        );
        assert!(
            !copper_golem_takes("", false, false, false),
            "nothing to take"
        );
        assert!(
            copper_golem_takes("minecraft:shears", true, false, true),
            "shears"
        );
        assert!(
            !copper_golem_takes("minecraft:shears", true, false, false),
            "no flower"
        );
        assert!(
            !copper_golem_takes("minecraft:honeycomb", false, false, true),
            "waxing is server-only"
        );
        assert!(
            !copper_golem_takes("minecraft:iron_axe", false, false, true),
            "scraping too"
        );
    }

    /// `AbstractHorse.handleEating` draws the particle only while the baby can
    /// still grow: `isBaby() && !isAgeLocked()` (#8590).
    #[test]
    fn a_locked_baby_draws_no_happy_villager() {
        let Some(mut horse) = mob("minecraft:horse") else {
            return;
        };
        horse.extra.age = -24000;
        assert!(ages_on_food(&horse, "minecraft:wheat"));
        horse.extra.age_locked = true;
        assert!(!ages_on_food(&horse, "minecraft:wheat"), "locked");
        horse.extra.age_locked = false;
        horse.extra.age = 0;
        assert!(!ages_on_food(&horse, "minecraft:wheat"), "adult");
        horse.extra.age = -24000;
        assert!(!ages_on_food(&horse, "minecraft:red_mushroom"), "no food");

        if let Some(mut llama) = mob("minecraft:llama") {
            llama.extra.age = -24000;
            assert!(ages_on_food(&llama, "minecraft:hay_block"));
            llama.extra.age_locked = true;
            assert!(!ages_on_food(&llama, "minecraft:hay_block"), "locked llama");
        }
        if let Some(mut camel) = mob("minecraft:camel")
            && camel.kind.food.iter().any(|f| f == "minecraft:cactus")
        {
            camel.extra.age = -24000;
            assert!(ages_on_food(&camel, "minecraft:cactus"));
            camel.extra.age_locked = true;
            assert!(!ages_on_food(&camel, "minecraft:cactus"), "locked camel");
        }
    }

    /// `Tadpole.feed` draws its particle on the client too, though a tadpole
    /// is never a baby; an age-locked one is not fed (#8554).
    #[test]
    fn a_tadpole_fed_a_slime_ball_draws_happy_villager() {
        let Some(mut tadpole) = mob("minecraft:tadpole") else {
            return;
        };
        assert!(!tadpole.is_baby());
        assert!(ages_on_food(&tadpole, "minecraft:slime_ball"));
        assert!(!ages_on_food(&tadpole, "minecraft:wheat"), "not frog food");
        tadpole.extra.age_locked = true;
        assert!(!ages_on_food(&tadpole, "minecraft:slime_ball"), "locked");
    }
}
