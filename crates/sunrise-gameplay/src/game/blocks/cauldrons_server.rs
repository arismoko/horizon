use super::*;

impl crate::game::ServerGame<'_> {
    /// `CauldronInteractions` for the held item aimed at a cauldron.
    pub fn use_on_cauldron(&mut self, pos: [i32; 3]) -> Result<sunrise_sim::cauldron::Outcome> {
        use sunrise_sim::cauldron::{self, Outcome};
        let registry = self.common.pack.assets.registry;
        let Some((state, under_water)) = self.common.cauldron_at(pos) else {
            return Ok(Outcome::Pass);
        };
        let Some(held) = self.common.held() else {
            return Ok(Outcome::Pass);
        };
        let removes_dye = cauldron::removes_dye(self.items(), &held.item);
        // `Stats.ITEM_USED.get(itemInHand.getItem())`: the item the hand held
        // before the interaction swaps it.
        let used_item = held.item.clone();
        let table = cauldron_interactions::snapshot();
        let items = self.items();
        let entry = table.select(&state.name, &held.item, removes_dye, |tag| {
            items.item_in_tag(tag, &held.item)
        });
        if let Some(cauldron::interactions::Entry::Mod(callback)) = &entry {
            return Ok(self.use_mod_cauldron(pos, under_water, callback));
        }

        let slot = self.common.held_slot();
        let player = self.native_acting_id();
        if self.common.transactions.is_empty() {
            use sunrise_gameplay_common::game::transactions::DeferredEffect;
            let mut hand = sunrise_gameplay_common::game::powder_snow::HeldSlot {
                game: self,
                slot,
                dropper: None,
                resource_lifetime: std::marker::PhantomData,
            };
            let result =
                cauldron::interact_selected(&state, under_water, entry.as_ref(), &mut hand);
            if result.outcome == Outcome::Pass {
                return Ok(Outcome::Pass);
            }
            if let Some(stat) = result.stat {
                self.defer_effect(DeferredEffect::CustomStat {
                    player,
                    key: stat,
                    amount: 1,
                });
            }
            // The bottle and bucket interactions award `ITEM_USED` right after
            // their cauldron statistic.
            if result.item_used {
                self.defer_effect(DeferredEffect::ItemStat {
                    player,
                    stat: "used",
                    item: used_item,
                    amount: 1,
                });
            }
            if let Some(block) = result.block
                && let Ok(new) = registry.resolve(&block)
            {
                self.level_set(pos, new)?;
                if result.lower_fill_level {
                    self.post_block_game_event("minecraft:block_change", pos, None, Some(new));
                }
            }
            if let Some(sound) = result.sound {
                self.defer_effect(DeferredEffect::Sound {
                    sound,
                    at: pos.map(|v| f64::from(v) + 0.5),
                    volume: 1.0,
                    pitch: 1.0,
                });
            }
            if let Some(event) = result.fluid_event {
                self.post_block_game_event(event, pos, None, None);
            }
            return Ok(result.outcome);
        }
        let outcome = self.storage_transaction(|game, _| {
            use sunrise_gameplay_common::game::transactions::DeferredEffect;
            game.journal_player(player);
            let mut hand = sunrise_gameplay_common::game::powder_snow::HeldSlot {
                game,
                slot,
                dropper: Some(player),
                resource_lifetime: std::marker::PhantomData,
            };
            let result =
                cauldron::interact_selected(&state, under_water, entry.as_ref(), &mut hand);
            if result.outcome == Outcome::Pass {
                return Err(());
            }
            if let Some(stat) = result.stat {
                game.defer_effect(DeferredEffect::CustomStat {
                    player,
                    key: stat,
                    amount: 1,
                });
            }
            // The bottle and bucket interactions award `ITEM_USED` right after
            // their cauldron statistic.
            if result.item_used {
                game.defer_effect(DeferredEffect::ItemStat {
                    player,
                    stat: "used",
                    item: used_item.clone(),
                    amount: 1,
                });
            }
            if let Some(block) = result.block {
                let new = registry.resolve(&block).map_err(|_| ())?;
                if !game.journal_set_block(pos, new) {
                    return Err(());
                }
                if result.lower_fill_level {
                    game.post_block_game_event("minecraft:block_change", pos, None, Some(new));
                }
            }
            if let Some(sound) = result.sound {
                game.defer_effect(DeferredEffect::Sound {
                    sound,
                    at: pos.map(|v| f64::from(v) + 0.5),
                    volume: 1.0,
                    pitch: 1.0,
                });
            }
            if let Some(event) = result.fluid_event {
                game.post_block_game_event(event, pos, None, None);
            }
            Ok(result.outcome)
        });
        Ok(outcome.unwrap_or(Outcome::Pass))
    }
}

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;
