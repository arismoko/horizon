//! `ServerPlaceRecipe` stages on a live mod `Menu` for checked, arbitrary slot roles.
use super::{Env, Kind, Menu, PLAYER, PostPlace, tidy, usable_for_crafting};
use super::{
    custom_plan::{MenuPlacement, PlacementRefusal as Refused},
    picker::Picker,
};
use crate::items::ItemStack;

#[cfg(test)]
#[path = "custom_tests.rs"]
mod tests;

/// What a slot's backing says it takes of one stack: the live handler's `capacity` for the
/// resource (NeoForge `ResourceHandlerSlot.getMaxStackSize`), asked once the planner has named
/// the slot and the stack. The placement takes the smaller of this and the menu's own limit.
#[derive(Clone, Debug)]
pub struct SlotCap {
    pub slot: usize,
    pub prototype: ItemStack,
    pub max: u32,
}

impl Menu {
    /// Shared vanilla calculateAmountToCraft, including an already matching grid.
    pub(super) fn recipe_place_amount(
        &self,
        inputs: &[usize],
        use_max: bool,
        matches: bool,
        biggest: i32,
    ) -> Option<i32> {
        Some(if use_max {
            biggest
        } else if matches {
            match inputs
                .iter()
                .filter_map(|slot| self.slot(*slot).map(|stack| stack.count))
                .min()
            {
                Some(count) => i32::try_from(count).ok()?.checked_add(1)?,
                None => i32::MAX,
            }
        } else {
            1
        })
    }

    /// Run `ServerPlaceRecipe.placeRecipe` stages directly on this live menu for an
    /// arbitrary-role mod layout. Returns `Ok(PostPlace::Nothing)` without mutating
    /// when survival inventory cannot clear the input grid (`testClearGrid`) or the
    /// grid already matches at maximum capacity, and `Ok(PostPlace::Ghost)` after
    /// clearing only when eligible ingredients are missing.
    pub fn place_custom_recipe(
        &mut self,
        env: &Env,
        plan: &MenuPlacement,
        use_max: bool,
        allow_drop: bool,
    ) -> Result<PostPlace, Refused> {
        self.place_custom_recipe_capped(env, plan, use_max, allow_drop, &[])
    }

    /// The stock the placement would draw on, one prototype (a single item) per distinct stack
    /// with its available amount: eligible inventory and sources, and the inputs it clears.
    fn recipe_prototypes(
        &self,
        env: &Env,
        plan: &MenuPlacement,
    ) -> Result<(Vec<ItemStack>, Vec<i32>), Refused> {
        let mut prototypes: Vec<ItemStack> = Vec::new();
        let mut amounts: Vec<i32> = Vec::new();
        let mut account = |stack: &ItemStack, simple: bool| -> Result<(), Refused> {
            if simple && !usable_for_crafting(stack) {
                return Ok(());
            }
            let count = i32::try_from(stack.count.min(Self::max_stack(env, stack)))
                .map_err(|_| Refused::InvalidLayout)?;
            if count == 0 {
                return Ok(());
            }
            if let Some(index) = prototypes
                .iter()
                .position(|p| p.same_item_same_components(stack))
            {
                amounts[index] = amounts[index]
                    .checked_add(count)
                    .ok_or(Refused::InvalidLayout)?;
            } else {
                prototypes.push(ItemStack {
                    count: 1,
                    ..stack.clone()
                });
                amounts.push(count);
            }
            Ok(())
        };
        for stack in self.state.containers[PLAYER][..36].iter().flatten() {
            account(stack, true)?;
        }
        for stack in self.state.sources.iter().flatten().flatten() {
            account(stack, true)?;
        }
        for &slot in &plan.inputs {
            if let Some(stack) = self.slot(slot) {
                account(stack, false)?;
            }
        }
        Ok((prototypes, amounts))
    }

    /// The (slot, stack) pairs the placement will weigh: each target slot of an ingredient with
    /// each stack that ingredient accepts and the menu lets the slot take. A game that backs
    /// slots with live handlers asks them about exactly these pairs, with its host, and hands the
    /// answers to [`Self::place_custom_recipe_capped`].
    pub fn custom_recipe_candidates(
        &self,
        env: &Env,
        plan: &MenuPlacement,
    ) -> Result<Vec<(usize, ItemStack)>, Refused> {
        let (prototypes, _) = self.recipe_prototypes(env, plan)?;
        let mut target = vec![0; plan.placement.ingredients.len()];
        for &(ingredient, slot) in &plan.cells {
            target[ingredient] = slot;
        }
        let mut pairs: Vec<(usize, ItemStack)> = Vec::new();
        for prototype in &prototypes {
            for (ingredient, &slot) in plan.placement.ingredients.iter().zip(&target) {
                if ingredient.test_in(env.loop_context, prototype)
                    && self.may_place(env, slot, prototype)
                    && !pairs
                        .iter()
                        .any(|(at, p)| *at == slot && p.same_item_same_components(prototype))
                {
                    pairs.push((slot, prototype.clone()));
                }
            }
        }
        Ok(pairs)
    }

    /// [`Self::place_custom_recipe`] with what each backed slot says it takes: a `(slot, stack)`
    /// the answers do not name keeps the menu's own limit.
    pub fn place_custom_recipe_capped(
        &mut self,
        env: &Env,
        plan: &MenuPlacement,
        use_max: bool,
        allow_drop: bool,
        caps: &[SlotCap],
    ) -> Result<PostPlace, Refused> {
        self.touch();
        if !matches!(self.kind(), Kind::Mod(menu) if menu.id == plan.menu) {
            return Err(Refused::Unavailable);
        }
        for &slot in &plan.clear {
            if self.slot(slot).is_some() && !self.may_pickup(env, slot) {
                return Err(Refused::Denied);
            }
        }
        if !allow_drop && !self.test_clear_slots(env, &plan.inputs) {
            return Ok(PostPlace::Nothing);
        }
        let (prototypes, mut amounts) = self.recipe_prototypes(env, plan)?;
        let ingredients = &plan.placement.ingredients;
        let mut target = vec![0; ingredients.len()];
        for &(ingredient, slot) in &plan.cells {
            target[ingredient] = slot;
        }
        let matches =
            plan.inputs
                .iter()
                .all(|slot| match plan.cells.iter().find(|(_, at)| at == slot) {
                    Some((ingredient, _)) => self.slot(*slot).is_some_and(|stack| {
                        ingredients[*ingredient].test_in(env.loop_context, stack)
                    }),
                    None => self.slot(*slot).is_none(),
                });
        let mut compatible = Vec::with_capacity(prototypes.len() * ingredients.len());
        let mut limits = Vec::with_capacity(prototypes.len() * ingredients.len());
        for prototype in &prototypes {
            for (ingredient, slot) in ingredients.iter().zip(&target) {
                let accepted = ingredient.test_in(env.loop_context, prototype);
                compatible.push(if accepted { i32::MAX } else { 0 });
                let allowed = accepted && self.may_place(env, *slot, prototype);
                let cap = if allowed {
                    let own = self
                        .slot_max(env, *slot, prototype)
                        .min(Self::max_stack(env, prototype));
                    caps.iter()
                        .find(|cap| {
                            cap.slot == *slot && cap.prototype.same_item_same_components(prototype)
                        })
                        .map_or(own, |cap| own.min(cap.max))
                } else {
                    0
                };
                limits.push(i32::try_from(cap).map_err(|_| Refused::InvalidLayout)?);
            }
        }
        let raw_available =
            super::can_craft_matching(&mut amounts, ingredients.len(), |candidate, ingredient| {
                compatible[candidate * ingredients.len() + ingredient]
            });
        if !raw_available {
            self.clear_slots(env, &plan.clear);
            if let Some(result) = plan.virtual_result {
                self.load_slot(result, None);
            }
            if plan.virtual_result.is_some() || self.mod_crafting().is_some() {
                self.recompute(env);
            }
            return Ok(PostPlace::Ghost);
        }
        let mut picker = Picker::new(&mut amounts, ingredients.len(), |candidate, ingredient| {
            limits[candidate * ingredients.len() + ingredient]
        });
        let biggest = picker.try_pick_all(i32::MAX, None);
        if biggest == 0 {
            return Err(Refused::Denied);
        }
        let amount = self
            .recipe_place_amount(&plan.inputs, use_max, matches, biggest)
            .ok_or(Refused::InvalidLayout)?;
        if amount > biggest {
            return Ok(PostPlace::Nothing);
        }
        let mut selected = Vec::new();
        if !picker.try_pick(amount, Some(&mut selected)) {
            return Ok(PostPlace::Nothing);
        }
        self.clear_slots(env, &plan.clear);
        if let Some(result) = plan.virtual_result {
            self.load_slot(result, None);
        }
        for (ingredient, candidate) in selected.into_iter().enumerate() {
            let prototype = &prototypes[candidate];
            let mut remaining = amount as u32;
            for stack in self.state.containers[PLAYER][..36]
                .iter_mut()
                .chain(self.state.sources.iter_mut().flatten())
            {
                if let Some(have) = stack
                    // Only eligible stock contributed to this exact prototype's
                    // budget. Cleared grid inputs remain usable even when named,
                    // enchanted or damaged; component equality preserves that budget.
                    && have.same_item_same_components(prototype)
                {
                    let taken = remaining.min(have.count);
                    have.count -= taken;
                    remaining -= taken;
                    tidy(stack);
                }
                if remaining == 0 {
                    break;
                }
            }
            let placed = (amount as u32) - remaining;
            if placed > 0 {
                self.load_slot(
                    target[ingredient],
                    Some(ItemStack {
                        count: placed,
                        ..prototype.clone()
                    }),
                );
            }
        }
        if plan.virtual_result.is_some() || self.mod_crafting().is_some() {
            self.recompute(env);
        }
        Ok(PostPlace::Nothing)
    }
}
