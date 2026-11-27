//! A mod's own advancement criteria triggers (docs/modding/src/how-to/native-mods.md,
//! "Advancement triggers"). An advancement of a content mod or a world's data
//! pack names a trigger of the mod's (`mymod:pipe_connected`); the tree keeps
//! it as it keeps vanilla's, and the mod fires it where its event happens.
use super::advancements::item_view;
use mod_api::{
    CriterionContext, CriterionTest, CriterionTestError, Identifier, PlayerRef,
    TriggerCriterionError,
};
use serde_json::Value;
use sunrise_sim::advancements as adv;

/// One open criterion of a mod's trigger, as the mod's test sees it.
struct NativeCriterion<'a> {
    conditions: &'a Value,
    world: &'a dyn adv::World,
}

impl CriterionContext for NativeCriterion<'_> {
    fn conditions(&self) -> &Value {
        self.conditions
    }

    fn item(&self, key: &str, stack: &mod_api::ItemStack) -> Result<bool, CriterionTestError> {
        let stack = sunrise_gameplay_common::game::api_values::stack_value(stack)
            .map_err(CriterionTestError::Stack)?;
        Ok(adv::item(
            &self.conditions[key],
            &item_view(&stack),
            self.world,
        ))
    }
}

impl crate::game::ServerGame<'_> {
    /// Runs the borrowed matcher for this custom trigger in the target player's level.
    pub(in crate::game) fn player_trigger_criterion(
        &mut self,
        player: PlayerRef,
        trigger: &Identifier,
        test: &mut dyn CriterionTest,
    ) -> Result<(), TriggerCriterionError> {
        if trigger.namespace().is_empty() || trigger.path().is_empty() {
            return Err(TriggerCriterionError::InvalidTrigger);
        }
        let trigger = trigger.as_str();
        if adv::is_builtin_trigger(trigger) {
            return Err(TriggerCriterionError::BuiltinTrigger);
        }
        self.with_criterion_player(player, |g| g.trigger_mod_criteria(trigger, test))?
    }

    /// Fire only `default_block_use` for a use a mod handled.
    pub(in crate::game) fn player_trigger_block_use(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
    ) -> Result<(), TriggerCriterionError> {
        self.with_criterion_player(player, |g| {
            g.trigger_block_use("minecraft:default_block_use", pos, None)
        })?
    }

    /// Fire only `item_used_on_block` against the pre-use snapshot.
    pub(in crate::game) fn player_trigger_item_used_on_block(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
        original: &sunrise_items::items::ItemStack,
    ) -> Result<(), TriggerCriterionError> {
        self.with_criterion_player(player, |g| {
            g.trigger_block_use("minecraft:item_used_on_block", pos, Some(original))
        })?
    }

    /// The same consume_item criterion path food and potions use, in the player's level.
    pub(in crate::game) fn player_trigger_consume_item(
        &mut self,
        player: PlayerRef,
        stack: &sunrise_items::items::ItemStack,
    ) -> Result<(), TriggerCriterionError> {
        self.with_criterion_player(player, |g| g.trigger_item("minecraft:consume_item", stack))
    }

    /// Fire in the player's own level; each predicate decides which world facts it needs.
    fn with_criterion_player<T>(
        &mut self,
        player: PlayerRef,
        fire: impl FnOnce(&mut Self) -> T,
    ) -> Result<T, TriggerCriterionError> {
        if !self.common.is_logical_server() {
            return Err(TriggerCriterionError::NotServer);
        }
        let target = self
            .player_handle_read(player)
            .and_then(|_| self.native_entity_target(player.entity()))
            .ok_or(TriggerCriterionError::PlayerUnavailable)?;
        self.for_player(target, fire)
            .ok_or(TriggerCriterionError::PlayerUnavailable)
    }

    /// `trigger` for a mod's trigger, for the player in place.
    fn trigger_mod_criteria(
        &mut self,
        trigger: &str,
        test: &mut dyn CriterionTest,
    ) -> Result<(), TriggerCriterionError> {
        self.load_advancements();
        if !self.common.adv.tree.by_trigger.contains_key(trigger) {
            return Ok(());
        }
        let me = self.player_entity_for(trigger);
        let player = adv::Entities {
            this: Some(&me),
            origin: me.position,
            tool: None,
            event_block: None,
            target: None,
            interacting: None,
        };
        let mut refused = None;
        // The level's random is lent to the tests and given back advanced, as a loot roll does.
        let mut random = self.common.world.random.clone();
        let loot = self.server.server_loot.0.get();
        let clocks = self.loot_clock_ticks();
        self.trigger_custom(trigger, |criterion, world| {
            if refused.is_some() {
                return false;
            }
            let context = NativeCriterion {
                conditions: &criterion.conditions,
                world,
            };
            match mod_api::__private::game::run_criterion_test(trigger, test, &context) {
                Ok(matched) => {
                    matched
                        && criterion.player.as_ref().is_none_or(|pred| {
                            let tags: &dyn sunrise_world::loot::LootTags = match &loot {
                                Some(data) => &**data,
                                None => &NoTags,
                            };
                            pred.test(
                                &player,
                                world,
                                &mut adv::LootRuntime {
                                    random: &mut random,
                                    tags,
                                    data: loot.as_deref(),
                                    clocks: &clocks,
                                },
                            )
                        })
                }
                Err(error) => {
                    refused = Some(error);
                    false
                }
            }
        });
        self.common.world.random = random;
        refused.map_or(Ok(()), Err)
    }
}

/// No tag registry is bound: no tag has members.
struct NoTags;
impl sunrise_world::loot::LootTags for NoTags {
    fn members(&self, _registry: &str, _tag: &str) -> Option<Vec<String>> {
        None
    }
    fn item_tag_names(&self) -> Option<&sunrise_world::loot::ItemTagNames> {
        None
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
