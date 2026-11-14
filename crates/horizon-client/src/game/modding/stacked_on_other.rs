//! `item_stacked_on_other` on the game's side: a pickup click, decided, fired and applied as one
//! action (the design's decide/fire/apply).
//!
//! The click reads what the hook is told (the cursor and the clicked slot's stack) and freezes
//! it. With no listener the plain click runs at once, with nothing allocated. With one, the game
//! opens an action savepoint, fires the hook with `HostSide` and the frozen stacks, validates the
//! typed answer once, checks that nothing the click read was written meanwhile, and only then
//! runs the click with the answer, inside the same savepoint. A stale read, an invalid answer, a
//! panicking listener or a refusal to admit leaves the click undone and every write the listener
//! made rolled back; the click then does nothing (the existing no-change result) and the reason
//! goes to the log.
use super::*;
use crate::game::api_values::{TryIntoApi, TryIntoEngine};
use crate::game::transactions::decision::{
    self, ActionTarget, GameHost, Identity, InvalidReason, MenuId, Outcome, Reply, Side,
    StaleReason,
};
use horizon_client::menu::{
    Env, Menu, PlanningRules, RuleAnswers, StackedClick, StackedOutcome, StoredRules,
};
use mod_api::item::menu::{StackResult, StackedOnOtherResult};
use std::{cell::RefCell, rc::Rc};

/// What a pickup click did: it ran, or the decision refused it and nothing changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum Pickup {
    Ran,
    Refused,
}

/// The click, run with the mods' answer (`None`: no listener, the plain click).
type Perform<'p> = Box<dyn FnOnce(&mut Game<'_>, Option<&StackedOutcome>) + 'p>;
/// What a click reads now, to compare with what it froze.
type Current<'p> = Box<dyn Fn(&Game<'_>) -> Option<StackedClick> + 'p>;

struct Shared<'p> {
    frozen: StackedClick,
    current: Current<'p>,
    perform: RefCell<Option<Perform<'p>>>,
    answer: RefCell<Option<StackedOutcome>>,
}

/// The decision's target: an id that names the route's menu (a second click on it while this one
/// is active is refused as busy), and what the click froze.
#[derive(Clone)]
struct PickupTarget<'p> {
    id: MenuId,
    shared: Rc<Shared<'p>>,
}

/// What the click read: the journaled-write epoch before the listeners ran.
struct PickupReads {
    writes: u64,
}

/// The one thing a pickup decision plans: that the click runs. The write set is the click's own.
struct PickupPlan;

impl<'a, 'g> ActionTarget<GameHost<'a, 'g>> for PickupTarget<'_> {
    type Reads = PickupReads;
    type View = PickupReads;
    type Plan = PickupPlan;
    type Frozen = PickupPlan;

    fn id(&self) -> MenuId {
        self.id
    }
    fn capture(&self, host: &GameHost<'a, 'g>) -> std::result::Result<PickupReads, Outcome> {
        Ok(PickupReads {
            writes: host.game.transactions.writes(),
        })
    }
    fn view(reads: &PickupReads) -> &PickupReads {
        reads
    }
    fn freeze(
        &self,
        _: &PickupReads,
        plan: PickupPlan,
    ) -> std::result::Result<PickupPlan, InvalidReason> {
        Ok(plan)
    }
    fn empty(_: &PickupPlan) -> bool {
        false
    }
    /// The conservative read set: any write the journals were asked to keep since the click
    /// froze its stacks, even one that restored what was there, makes it stale; and what the
    /// click would read now must still be what it froze.
    fn validate(
        &self,
        host: &GameHost<'a, 'g>,
        reads: &PickupReads,
    ) -> std::result::Result<(), Outcome> {
        if host.game.transactions.writes() != reads.writes {
            return Err(Outcome::Stale(StaleReason::Mutation));
        }
        match (self.shared.current)(host.game) {
            Some(now) if now == self.shared.frozen => Ok(()),
            _ => Err(Outcome::Stale(StaleReason::Snapshot)),
        }
    }
    fn apply(&self, host: &mut GameHost<'a, 'g>, _: PickupPlan) {
        let perform = self.shared.perform.take().expect("a pickup runs once");
        let answer = self.shared.answer.take();
        perform(host.game, answer.as_ref());
    }
}

/// Where a pickup decision's menu is named: a route's own id, stable for as long as the route's
/// menu lives, so that a click made from inside the action finds it busy.
pub(in crate::game) fn local_pickup_id() -> MenuId {
    static ID: std::sync::OnceLock<MenuId> = std::sync::OnceLock::new();
    *ID.get_or_init(MenuId::fresh)
}

impl Game<'_> {
    /// Whether any listener answers `item_stacked_on_other`: the one check a plain click makes.
    pub(in crate::game) fn stacked_listened() -> bool {
        mod_api::__private::listened(mod_api::HookId::ItemStackedOnOther)
    }

    /// The pickup click whose frozen stacks are `frozen`, decided by the mods and then run by
    /// `perform`, which is given their answer. `current` reads the same stacks again, for the
    /// check that nothing changed them. `route` names the menu clicked.
    ///
    /// With no listener this is `perform(self, None)` and nothing else: no transaction, no
    /// decision, no allocation.
    pub(in crate::game) fn stacked_pickup<'p>(
        &mut self,
        route: MenuId,
        frozen: StackedClick,
        current: impl Fn(&Game<'_>) -> Option<StackedClick> + 'p,
        perform: impl FnOnce(&mut Game<'_>, Option<&StackedOutcome>) + 'p,
    ) -> Pickup {
        if !Self::stacked_listened() {
            perform(self, None);
            return Pickup::Ran;
        }
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return Pickup::Refused;
        };
        let Some(entity) = frozen
            .player_entity_id
            .and_then(|id| u32::try_from(id).ok())
        else {
            return Pickup::Refused;
        };
        let actor = mod_api::EntityRef::new(epoch, entity);
        let identity = Identity {
            side: Side::Server,
            world: epoch,
            actor,
            target: actor,
        };
        let shared = Rc::new(Shared {
            frozen,
            current: Box::new(current),
            perform: RefCell::new(Some(Box::new(perform))),
            answer: RefCell::new(None),
        });
        let target = PickupTarget {
            id: route,
            shared: shared.clone(),
        };
        // The action's savepoint needs a transaction to nest in: the open one, or this one.
        let seal = self.transaction_open();
        let outcome = {
            let mut host = GameHost {
                game: &mut *self,
                identity,
            };
            let fired = shared.clone();
            decision::run(
                &mut host,
                &target,
                |_, _| PickupPlan,
                move |host| fire(host.game, &fired),
            )
        };
        self.transaction_close(&seal, matches!(outcome, Ok(Outcome::Applied)));
        match outcome {
            Ok(Outcome::Applied) => Pickup::Ran,
            Ok(other) => {
                log::warn!(
                    "item_stacked_on_other: the click was refused ({other:?}); nothing changed"
                );
                Pickup::Refused
            }
            Err(admission) => {
                log::warn!(
                    "item_stacked_on_other: the click was not admitted ({admission:?}); nothing changed"
                );
                Pickup::Refused
            }
        }
    }
}

/// Fire the hook with the frozen stacks and validate what it answers. The answer is kept for the
/// click; a bad one refuses it.
fn fire(game: &mut Game<'_>, shared: &Shared<'_>) -> Reply {
    let frozen = &shared.frozen;
    let (Ok(carried), Ok(clicked)) = (frozen.carried.try_into_api(), frozen.clicked.try_into_api())
    else {
        return Reply::Invalid(InvalidReason::Plan);
    };
    let event = mod_api::item::menu::StackedOnOther::new(
        frozen.player,
        frozen.player_entity_id,
        frozen.slot,
        frozen.container,
        frozen.container_slot,
        frozen.primary,
        carried,
        clicked,
    );
    let mut result = StackedOnOtherResult::default();
    let mut canceled = false;
    game.native_item_stacked_on_other(&event, &mut result, &mut canceled);
    match answer(game, canceled, &result) {
        Ok(answer) => {
            *shared.answer.borrow_mut() = Some(answer);
            Reply::Allow
        }
        Err(reason) => Reply::Invalid(reason),
    }
}

/// The typed answer, validated once: a stack may be set only by a cancelled click that is also
/// handled, and a set stack must be a valid one (a known item, a count within its maximum).
fn answer(
    game: &Game<'_>,
    canceled: bool,
    result: &StackedOnOtherResult,
) -> std::result::Result<StackedOutcome, InvalidReason> {
    let sets = |r: &StackResult| matches!(r, StackResult::Set(_));
    if !(canceled && result.handled) && (sets(&result.carried) || sets(&result.clicked)) {
        return Err(InvalidReason::Plan);
    }
    let items = game.items();
    let convert = |r: &StackResult| match r {
        StackResult::Keep => Ok(None),
        StackResult::Set(stack) => stack
            .try_into_engine(items)
            .map(Some)
            .map_err(|_| InvalidReason::Count),
    };
    Ok(StackedOutcome {
        canceled,
        handled: result.handled,
        carried: convert(&result.carried)?,
        clicked: convert(&result.clicked)?,
    })
}

/// What an applied click writes belongs to the action: an outer abort puts it back.
impl Game<'_> {
    /// Keep guest `conn`'s inventory, armor, off hand, equipment and cursor (and their crafting
    /// grid, ender chest and mod menu storage), as they are, in the open transaction.
    pub(in crate::game) fn journal_guest_click(&mut self, conn: u32) {
        let Some(uuid) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| g.uuid)
        else {
            return;
        };
        self.native_as_player(uuid, |game| game.journal_player(uuid));
        if let Some(player) = self.player(uuid) {
            self.journal_menus(player);
        }
    }
}

/// The local menu's route (`menu_ops`).
impl Game<'_> {
    /// The clicks `menu_ops` was asked for, when they are one pickup of a slot of the local menu
    /// and the mods listen: `Some(whether it was handled)`. `None`: the plain clicks.
    pub(in crate::game) fn stacked_menu_ops(
        &mut self,
        ops: &[(Option<usize>, i32, horizon_client::menu::Input)],
    ) -> Option<bool> {
        match *ops {
            [(Some(slot), button, horizon_client::menu::Input::Pickup)]
                if self.remote.is_none() =>
            {
                self.stacked_menu_pickup(slot, button)
            }
            _ => None,
        }
    }

    /// A single pickup click on the open menu's game slot `slot`, decided by the mods first when
    /// any listens: `Some(whether the click was handled)`. `None` when none listens, and the
    /// caller makes the plain click.
    pub(in crate::game) fn stacked_menu_pickup(
        &mut self,
        slot: usize,
        button: i32,
    ) -> Option<bool> {
        use horizon_client::menu::Input;
        if !Self::stacked_listened() || slot == 200 {
            return None;
        }
        // Plan: read what the click finds and ask the slot rules for what the click would
        // consult (once for each distinct question, on a copy of the menu, for each way the hook
        // can leave the click), keeping nothing: the session is only a view of the game.
        let answers = Rc::new(RuleAnswers::default());
        let mut found = None;
        let modeled = self.menu_session(|menu, env, to_menu| {
            let menu_slot = to_menu(slot)? as i32;
            found = menu.stacked_click(env, menu_slot, button, Input::Pickup);
            if found.is_some() {
                plan_click(menu, env, menu_slot, button, &answers);
            }
            Some(false)
        });
        let frozen = found.filter(|_| modeled)?;
        let handled = Rc::new(std::cell::Cell::new(false));
        let result = handled.clone();
        let carried = frozen.carried.clone();
        let stored = answers.clone();
        let pickup = self.stacked_pickup(
            local_pickup_id(),
            frozen.clone(),
            // What the click reads of the game itself: the cursor. The rest is the journals'.
            move |game| (game.carried == carried).then(|| frozen.clone()),
            move |game, answer| {
                // The click's own writes belong to the action: an outer abort puts them back.
                let player = game.native_acting_id();
                game.journal_player(player);
                if let Some(key) = game.container {
                    game.journal_block_entity(key);
                }
                result.set(game.menu_session(|menu, env, to_menu| {
                    // The click asks no one: it uses what the plan was told.
                    let rules = StoredRules(&stored);
                    let env = Env {
                        slot_rules: Some(&rules),
                        stacked: answer,
                        ..*env
                    };
                    let menu_slot = to_menu(slot)? as i32;
                    menu.click(&env, menu_slot, button, Input::Pickup)
                        .map(|()| true)
                        .ok()
                        .or(Some(false))
                }));
            },
        );
        // A refused click was handled: nothing of it is kept, and no other click is made.
        Some(pickup == Pickup::Refused || handled.get())
    }
}

/// Ask the slot rules for what a pickup click consults, for each way the hook can leave it: the
/// bundle override and the plain pickup (not cancelled), and the plain pickup (cancelled, not
/// handled). A cancelled, handled click only sets stacks and consults nothing. The click runs on
/// copies of `menu`; the asks, and nothing else, are kept in `answers`.
pub(in crate::game) fn plan_click(
    menu: &Menu,
    env: &Env,
    slot: i32,
    button: i32,
    answers: &RuleAnswers,
) {
    let planning = PlanningRules {
        rules: env.slot_rules,
        answers,
    };
    for outcome in [
        StackedOutcome::default(),
        StackedOutcome {
            canceled: true,
            ..StackedOutcome::default()
        },
    ] {
        let env = Env {
            slot_rules: Some(&planning),
            stacked: Some(&outcome),
            ..*env
        };
        let _ = menu
            .clone()
            .click(&env, slot, button, horizon_client::menu::Input::Pickup);
    }
}
