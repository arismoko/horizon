//! Loot rolls with a mod listening to `enchanted_entity_loot`.

use super::commands::Target;
use crate::game::ServerGame;
use mod_api::entity::health::HurtSource;
use mod_api::{EntityRef, LivingRef};
use sunrise_core::random::LegacyRandom;
use sunrise_gameplay_common::game::creatures::{death_source::death_hurt_source, to_loot};
use sunrise_items::items::{ItemStack, Items};
use sunrise_items::random_sequences::RandomSequences;
use sunrise_sim::mobs::drops::{DeathRolls, roll_asked};

/// A table's rolls with a mod listening to `enchanted_entity_loot`: the
/// randoms stay in the game, where the listener's host reaches them.
pub(in crate::game) struct ListenedRolls<'g, 'a, 'h> {
    game: &'g mut crate::game::ServerGame<'a>,
    entity: LivingRef,
    source: &'h HurtSource<'h>,
    /// `LootContextParams.ATTACKING_ENTITY`.
    attacker: Option<EntityRef>,
}

impl<'g, 'a, 'h> ListenedRolls<'g, 'a, 'h> {
    /// A death's rolls: its attacker is the blow's causing entity.
    pub(in crate::game::server_side) fn new(
        game: &'g mut crate::game::ServerGame<'a>,
        entity: LivingRef,
        source: &'h HurtSource<'h>,
    ) -> Self {
        let attacker = u32::try_from(source.causing)
            .ok()
            .filter(|id| *id != 0)
            .map(|id| EntityRef::new(entity.entity().epoch(), id));
        Self {
            game,
            entity,
            source,
            attacker,
        }
    }
}

impl DeathRolls for ListenedRolls<'_, '_, '_> {
    fn sequences(&mut self) -> &mut RandomSequences {
        &mut self.game.server.rand_sequences
    }
    fn level_random(&mut self) -> &mut LegacyRandom {
        &mut self.game.world.random
    }
    fn asks_enchanted_level(&self) -> bool {
        true
    }
    fn enchanted_level(&mut self, enchantment: &str, level: i32) -> i32 {
        let Ok(id) = mod_api::Identifier::parse(enchantment) else {
            return level;
        };
        let mut asked = level.max(0) as u32;
        self.game.native_enchanted_entity_loot(
            self.entity,
            self.source,
            self.attacker,
            &id,
            &mut asked,
        );
        i32::try_from(asked).unwrap_or(i32::MAX)
    }
}

/// `/loot ... kill`'s roll (`LootCommand.dropKillLoot`).
pub(in crate::game) struct KillRoll<'a> {
    /// The target's type and its table.
    pub(in crate::game) kind: &'a str,
    pub(in crate::game) table: &'a str,
    pub(in crate::game) target: Target,
    /// `LootContextParams.ATTACKING_ENTITY`: the command's source.
    pub(in crate::game) killer: Option<Target>,
    /// `LootContextParams.LAST_DAMAGE_PLAYER` is set.
    pub(in crate::game) by_player: bool,
    pub(in crate::game) held: Option<&'a ItemStack>,
    pub(in crate::game) keep_empty: bool,
}

impl ServerGame<'_> {
    /// The entity `/loot ... kill` names, with a mod to ask: `EnchantedEntityLootEvent`
    /// needs a `LivingEntity`, and a table of any other entity is not asked about.
    fn listened_kill_target(&self, target: Target) -> Option<LivingRef> {
        if !mod_api::__private::listened(mod_api::HookId::EnchantedEntityLoot) {
            return None;
        }
        let (Target::Mob(_) | Target::Stand(_) | Target::Dragon(_) | Target::Guest(_)) = target
        else {
            return None;
        };
        self.effect_living_ref(Some(self.network_entity_id(target)?))
    }
    /// The drops of `/loot ... kill`, from the table's random sequence, else
    /// the level random. It reads levels as a death does, and a mod listening
    /// to `enchanted_entity_loot` is asked: the damage is `magic()`, as
    /// vanilla's, whose source names nobody, so the attacker is the command's.
    pub(in crate::game::server_side) fn roll_kill_loot(
        &mut self,
        kill: KillRoll<'_>,
    ) -> Vec<ItemStack> {
        let KillRoll {
            kind,
            table,
            target,
            killer,
            by_player,
            held,
            keep_empty,
        } = kill;
        // LootCommand.dropKillLoot supplies the selected live mob as THIS_ENTITY.
        let live_target = match target {
            Target::Mob(id) => self
                .common
                .mobs
                .iter()
                .any(|mob| mob.id == id)
                .then(|| self.target_loot_entity(target)),
            _ => None,
        };
        let loot = live_target
            .as_ref()
            .and_then(|_| self.server.server_loot.0.get());
        let matcher = |predicate: &serde_json::Value, entity: &sunrise_world::loot::LootEntity| {
            loot.as_ref()
                .is_some_and(|data| data.entity_matches(predicate, Some(entity)))
        };
        let live_target = live_target.as_ref().filter(|_| loot.is_some());
        // The listeners get the game, so the roll holds its own share of the items.
        let listened = self
            .common
            .server_items
            .clone()
            .zip(self.listened_kill_target(target));
        let sequence = server_world_items!(self).loot_sequence(table);
        let Some((items, entity)) = listened else {
            let items = server_world_items!(self);
            return match sequence {
                Some(id) => {
                    let random = self.server.rand_sequences.get(&id, self.common.level.seed);
                    if let Some(target) = live_target {
                        items.entity_drops_with_target(
                            kind, by_player, held, target, &matcher, random, keep_empty,
                        )
                    } else {
                        items.entity_drops_for_command(kind, by_player, held, random, keep_empty)
                    }
                }
                None => {
                    if let Some(target) = live_target {
                        items.entity_drops_with_target(
                            kind,
                            by_player,
                            held,
                            target,
                            &matcher,
                            &mut self.common.world.random,
                            keep_empty,
                        )
                    } else {
                        items.entity_drops_for_command(
                            kind,
                            by_player,
                            held,
                            &mut self.common.world.random,
                            keep_empty,
                        )
                    }
                }
            };
        };
        let epoch = entity.entity().epoch();
        let attacker = killer
            .and_then(|t| self.network_entity_id(t))
            .map(|id| EntityRef::new(epoch, id as u32));
        let seed = self.common.level.seed;
        let source = death_hurt_source("minecraft:magic", 0);
        let mut rolls = ListenedRolls {
            game: self,
            entity,
            source: &source,
            attacker,
        };
        let items: &Items = &items;
        match sequence {
            Some(id) => roll_asked(
                &mut rolls,
                |r| r.sequences().get(&id, seed),
                |random, ask| {
                    if let Some(target) = live_target {
                        items.entity_drops_asked_with_target(
                            kind, by_player, held, target, &matcher, random, ask, keep_empty,
                        )
                    } else {
                        items.entity_drops_asked(kind, by_player, held, random, ask, keep_empty)
                    }
                },
            ),
            None => roll_asked(
                &mut rolls,
                |r| r.level_random(),
                |random, ask| {
                    if let Some(target) = live_target {
                        items.entity_drops_asked_with_target(
                            kind, by_player, held, target, &matcher, random, ask, keep_empty,
                        )
                    } else {
                        items.entity_drops_asked(kind, by_player, held, random, ask, keep_empty)
                    }
                },
            ),
        }
    }
}

/// The parameters a caller supplies around a roll besides its entity: the
/// ORIGIN and level a mod's conditions read, and the TOOL.
pub(in crate::game) struct CallerLoot {
    place: sunrise_world::loot::LootPlace,
    query: std::rc::Rc<sunrise_world::loot::query::LootQuery>,
}

impl CallerLoot {
    /// Runs `roll` with these parameters in scope for every table it rolls.
    pub(in crate::game::server_side) fn with<T>(self, roll: impl FnOnce() -> T) -> T {
        sunrise_world::loot::query::with_query(self.query, || {
            sunrise_world::loot::with_native_loot_place(self.place, roll)
        })
    }
}

impl ServerGame<'_> {
    /// A roll's `origin` (None when its parameter set has none) and `tool`,
    /// taken before the roll borrows the game.
    pub(in crate::game::server_side) fn caller_loot(
        &self,
        origin: Option<[f64; 3]>,
        tool: Option<sunrise_world::loot::LootStack>,
    ) -> CallerLoot {
        CallerLoot {
            place: sunrise_world::loot::LootPlace {
                origin,
                ..self.common.loot_place_at([0.0; 3], None)
            },
            query: std::rc::Rc::new(sunrise_world::loot::query::LootQuery::new(None, tool)),
        }
    }

    /// What a loot table reads of `target` as `THIS_ENTITY`: a mob's facts, a
    /// player's worn and held items, and the entity's type for the rest.
    pub(in crate::game::server_side) fn target_loot_entity(
        &mut self,
        target: Target,
    ) -> sunrise_world::loot::LootEntity {
        let mut entity = match target {
            Target::Mob(id) => self
                .common
                .mobs
                .iter()
                .find(|mob| mob.id == id)
                .map(|mob| {
                    let vehicle = mob
                        .extra
                        .mount
                        .and_then(|mount| self.common.mobs.iter().find(|other| other.id == mount))
                        .map(|other| sunrise_world::loot::LootEntity::new(&other.kind.id));
                    sunrise_sim::mobs::drops::loot_entity_with_variants(
                        mob,
                        vehicle,
                        Some(self.world_variants()),
                    )
                })
                .unwrap_or_else(|| sunrise_world::loot::LootEntity::new(&self.target_type(target))),
            _ => {
                let mut entity = sunrise_world::loot::LootEntity {
                    kind: self.target_type(target),
                    living: target.is_living(),
                    ..Default::default()
                };
                if entity.living {
                    entity.equipment = mod_api::EquipmentSlot::ALL
                        .into_iter()
                        .zip(sunrise_world::loot::EQUIPMENT_SLOTS)
                        .filter_map(|(slot, name)| {
                            let stack = self.equipment_stack_for(target, slot)?;
                            Some((name.to_owned(), to_loot(&stack.project())))
                        })
                        .collect();
                }
                entity
            }
        };
        if entity.nbt.is_none()
            && self
                .server
                .server_loot
                .0
                .get()
                .is_some_and(|data| data.copies_entity_nbt())
        {
            entity.nbt = self.command_target_nbt(target).map(std::sync::Arc::new);
        }
        entity
    }

    /// One roll of `table` (`LootTable.getRandomItems`) with `origin` as ORIGIN (a
    /// parameter set without one passes None), with `held` as
    /// TOOL and `this` as THIS_ENTITY when given, from the table's random
    /// sequence, else the level random. The stacks come split to their stack
    /// sizes; None for a missing table.
    pub(in crate::game::server_side) fn roll_table_at(
        &mut self,
        table: &str,
        origin: Option<[f64; 3]>,
        held: Option<&ItemStack>,
        this: Option<&sunrise_world::loot::LootEntity>,
    ) -> Option<Vec<ItemStack>> {
        let loot = self.server.server_loot.0.get();
        let matcher = |predicate: &serde_json::Value, entity: &sunrise_world::loot::LootEntity| {
            loot.as_ref()
                .is_some_and(|data| data.entity_matches(predicate, Some(entity)))
        };
        let this = this.map(|entity| {
            (
                entity,
                &matcher as &dyn Fn(&serde_json::Value, &sunrise_world::loot::LootEntity) -> bool,
            )
        });
        self.caller_loot(origin, None).with(|| {
            let mut random = self.common.world.random.clone();
            let mut sequences = std::mem::take(&mut self.server.rand_sequences);
            let drops = {
                let level = crate::game::advancements::View(&*self);
                let read = |predicate: &serde_json::Value, at: [f64; 3]| {
                    sunrise_sim::advancements::location(predicate, at, &level)
                };
                let location = origin.map(|at| sunrise_sim::advancements::loot_location(at, &read));
                let items = server_world_items!(self);
                match items.loot_sequence(table) {
                    Some(sequence) => {
                        let random = sequences.get(&sequence, self.common.level.seed);
                        items.roll_loot_table(table, held, this, location, random)
                    }
                    None => items.roll_loot_table(table, held, this, location, &mut random),
                }
            };
            self.common.world.random = random;
            self.server.rand_sequences = sequences;
            drops
        })
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;
