//! Engine menu records and slot access.
use super::ItemStack;
/// Operations on the clicked slot, including its placement and take rules.
pub trait SlotView {
    /// Run against staged owner state; commit all changes only when accepted.
    fn with_staged(&mut self, edit: &mut dyn FnMut(&mut dyn SlotView) -> bool) -> bool;
    fn items(&self) -> &horizon_items::items::Items;
    fn item(&self) -> Option<ItemStack>;
    fn may_place(&self, stack: &ItemStack) -> bool;
    fn may_pickup(&self) -> bool;
    fn allow_modification(&self) -> bool;
    fn safe_insert(&mut self, stack: Option<ItemStack>, count: u32) -> Option<ItemStack>;
    fn safe_take(&mut self, count: u32, max: u32) -> Option<ItemStack>;
}
/// A pickup click as `item_stacked_on_other` is asked about it: the cursor and the clicked slot's
/// stack as the click finds them, before any listener runs.
#[derive(Clone, Debug, PartialEq)]
pub struct StackedClick {
    pub player: Option<u128>,
    pub player_entity_id: Option<i32>,
    pub slot: usize,
    pub container: usize,
    pub container_slot: usize,
    pub primary: bool,
    pub carried: Option<ItemStack>,
    pub clicked: Option<ItemStack>,
}
/// What the mods decided about a pickup click (`item_stacked_on_other`), validated by the game
/// before the click: `canceled` skips the bundle override, and a cancelled click that is also
/// `handled` is done, leaving the cursor and the slot as the `Some` stacks say.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StackedOutcome {
    pub canceled: bool,
    pub handled: bool,
    /// The cursor after a cancelled, handled click.
    pub carried: Option<Option<ItemStack>>,
    /// The clicked slot after a cancelled, handled click.
    pub clicked: Option<Option<ItemStack>>,
}
/// `menu_slot_rules` inside a menu click, which cannot reach the game: the game lends the menu
/// one for the click. A click asks it once for each slot and stack it consults.
pub trait SlotRules {
    /// Whether a part answers the menu type `menu`; a menu type none answers is never asked.
    fn listens(&self, menu: &str) -> bool;
    /// The slot's own answers arrive in `may_place` and `max_stack`; the parts may change them.
    fn rule(
        &self,
        menu: &str,
        slot: usize,
        stack: &ItemStack,
        may_place: &mut bool,
        max_stack: &mut u32,
    );
}

/// What `AnvilUpdateEvent` asks about once `createResult` made vanilla's
/// result with a left input, and what the mods may change: the `output`,
/// its level `cost` and `material_cost`, the units of `right` used (0 or
/// more than it holds: all of it), each starting at vanilla's.
#[derive(Clone, Debug, PartialEq)]
pub struct AnvilUpdate {
    pub left: ItemStack,
    pub right: Option<ItemStack>,
    /// The rename box's text, `None` when the client sent none.
    pub name: Option<String>,
    /// Vanilla's result for these inputs (`getVanillaResult`), whatever
    /// the mods set.
    pub vanilla: AnvilVanilla,
    pub output: Option<ItemStack>,
    /// Signed level cost, preserved as set by `AnvilUpdateEvent.setXpCost`.
    /// A negative cost prevents taking the result (as does zero), not a level refund.
    pub xp_cost: i32,
    pub material_cost: i32,
}
/// `AnvilUpdateEvent.VanillaResult`: the output, level cost and material
/// cost `createResult` made.
#[derive(Clone, Debug, PartialEq)]
pub struct AnvilVanilla {
    pub output: Option<ItemStack>,
    pub xp_cost: i32,
    pub material_cost: i32,
}
impl AnvilUpdate {
    /// The question about `vanilla`, the result `createResult` made from
    /// these inputs and name: the output, level cost and material cost
    /// start at its own.
    pub fn new(
        left: ItemStack,
        right: Option<ItemStack>,
        name: Option<String>,
        vanilla: &crate::stations::AnvilResult,
    ) -> Self {
        let vanilla = AnvilVanilla {
            output: vanilla.result.clone(),
            xp_cost: vanilla.cost,
            material_cost: i32::try_from(vanilla.repair_item_count_cost).unwrap_or(i32::MAX),
        };
        Self {
            left,
            right,
            name,
            output: vanilla.output.clone(),
            xp_cost: vanilla.xp_cost,
            material_cost: vanilla.material_cost,
            vanilla,
        }
    }

    /// The mods' answer, in place of `result`: a cancel leaves no result at
    /// cost 0; otherwise their output, signed level cost and nonnegative
    /// material cost. A changed output is no longer only a rename.
    pub fn answer(self, cancel: bool, result: &mut crate::stations::AnvilResult) {
        if cancel {
            *result = crate::stations::AnvilResult::default();
            return;
        }
        result.only_renaming &= self.output == self.vanilla.output;
        result.cost = self.xp_cost;
        result.repair_item_count_cost = self.material_cost.max(0) as u32;
        result.result = self.output;
    }
}
/// What `GrindstoneEvent.OnPlaceItem` asks about once the inputs changed,
/// and what the mods may set: an `output` (none until a mod sets one) and,
/// with it, `xp` (-1 for vanilla's experience). `vanilla_output` is the
/// vanilla's result, to read; set `output` to replace it.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct GrindstonePlace {
    pub top: Option<ItemStack>,
    pub bottom: Option<ItemStack>,
    pub vanilla_output: Option<ItemStack>,
    pub output: Option<ItemStack>,
    pub xp: i32,
}
/// A grindstone result taken, as `GrindstoneEvent.OnTakeItem` reports it:
/// the inputs, the experience (-1 for vanilla's, or what OnPlaceItem set)
/// and what the inputs hold afterwards (empty, as vanilla's).
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct GrindstoneTake {
    pub top: Option<ItemStack>,
    pub bottom: Option<ItemStack>,
    pub xp: i32,
    pub new_top: Option<ItemStack>,
    pub new_bottom: Option<ItemStack>,
}
/// An anvil craft as `AnvilCraftEvent` reports it: the inputs and the result
/// taken, before the crafting changed anything.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct AnvilCraft {
    pub left: Option<ItemStack>,
    pub right: Option<ItemStack>,
    pub output: ItemStack,
}

/// The slot-rule answers a click asked while it was planned, kept for the click that runs: a
/// decision plans first (asking the mods once for each slot and stack the click consults, with
/// the slot's own answers as the question) and then validates and applies from these answers
/// alone. An ask the plan did not make is not made later: the slot's own answer stands.
#[derive(Default)]
pub struct RuleAnswers {
    answers: std::cell::RefCell<Vec<RuleAnswer>>,
    asked: std::cell::Cell<usize>,
    missed: std::cell::Cell<usize>,
}

struct RuleAnswer {
    menu: String,
    slot: usize,
    stack: ItemStack,
    question: (bool, u32),
    answer: (bool, u32),
}

impl RuleAnswers {
    /// How many times the mods were asked while planning.
    pub fn asked(&self) -> usize {
        self.asked.get()
    }
    /// How many asks the click made that the plan had not (answered by the slot itself).
    pub fn missed(&self) -> usize {
        self.missed.get()
    }
    fn find(
        &self,
        menu: &str,
        slot: usize,
        stack: &ItemStack,
        question: (bool, u32),
    ) -> Option<(bool, u32)> {
        self.answers
            .borrow()
            .iter()
            .find(|a| {
                a.slot == slot && a.menu == menu && a.question == question && a.stack == *stack
            })
            .map(|a| a.answer)
    }
}

/// Plans with the game's rules: asks them, once for each distinct question, and keeps the answer.
pub struct PlanningRules<'a> {
    pub rules: Option<&'a dyn SlotRules>,
    pub answers: &'a RuleAnswers,
}

impl SlotRules for PlanningRules<'_> {
    fn listens(&self, menu: &str) -> bool {
        self.rules.is_some_and(|rules| rules.listens(menu))
    }
    fn rule(
        &self,
        menu: &str,
        slot: usize,
        stack: &ItemStack,
        may_place: &mut bool,
        max_stack: &mut u32,
    ) {
        let question = (*may_place, *max_stack);
        if let Some(found) = self.answers.find(menu, slot, stack, question) {
            (*may_place, *max_stack) = found;
            return;
        }
        if let Some(rules) = self.rules {
            rules.rule(menu, slot, stack, may_place, max_stack);
        }
        self.answers.asked.set(self.answers.asked.get() + 1);
        self.answers.answers.borrow_mut().push(RuleAnswer {
            menu: menu.to_owned(),
            slot,
            stack: stack.clone(),
            question,
            answer: (*may_place, *max_stack),
        });
    }
}

/// Runs with the answers a plan kept, and asks no one.
pub struct StoredRules<'a>(pub &'a RuleAnswers);

impl SlotRules for StoredRules<'_> {
    fn listens(&self, _: &str) -> bool {
        // The plan asked only where a part answers; where it asked nothing, nothing is asked.
        !self.0.answers.borrow().is_empty()
    }
    fn rule(
        &self,
        menu: &str,
        slot: usize,
        stack: &ItemStack,
        may_place: &mut bool,
        max_stack: &mut u32,
    ) {
        match self.0.find(menu, slot, stack, (*may_place, *max_stack)) {
            Some(found) => (*may_place, *max_stack) = found,
            None => self.0.missed.set(self.0.missed.get() + 1),
        }
    }
}
