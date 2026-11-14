//! One decision for a hook that is not a pickup click (`stacked_on_other.rs` is the pickup's): the
//! mod menu's shift-click (`menu_quick_move`) and the click on a menu a mod opened
//! (`menu_clicked`). The same coordinator, the same savepoint, the same refusal.
//!
//! With no listener the caller makes the ordinary call and nothing here runs. With one, the game
//! opens an action savepoint, fires the hook, checks that nothing the action read was written
//! while the listener ran (`Epoch`), and only then performs the action, inside the same
//! savepoint. A stale read, an invalid answer, a panicking listener and a busy menu leave it
//! undone with every write the listener made rolled back.
use super::*;
use crate::game::transactions::decision::{
    self, ActionTarget, GameHost, Identity, InvalidReason, MenuId, Outcome, Reply, Side,
    StaleReason,
};
use mod_api::EntityRef;
use std::{cell::RefCell, rc::Rc};

/// What a decision reads of the journals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum Epoch {
    /// Nothing: the action reads no state a listener could write.
    Off,
    /// A player's own state (inventory, cursor, menu storage). A write to another block, which a
    /// mod's move legitimately makes, does not make the decision stale.
    Player,
}

type Fresh<'p> = Box<dyn Fn(&Game<'_>) -> bool + 'p>;
type Perform<'p> = Box<dyn FnOnce(&mut Game<'_>) + 'p>;

struct Shared<'p> {
    epoch: Epoch,
    fresh: Fresh<'p>,
    perform: RefCell<Option<Perform<'p>>>,
}

#[derive(Clone)]
struct Action<'p> {
    id: MenuId,
    shared: Rc<Shared<'p>>,
}

struct Reads {
    player_writes: u64,
}

struct Plan;

impl<'a, 'g> ActionTarget<GameHost<'a, 'g>> for Action<'_> {
    type Reads = Reads;
    type View = Reads;
    type Plan = Plan;
    type Frozen = Plan;

    fn id(&self) -> MenuId {
        self.id
    }
    fn capture(&self, host: &GameHost<'a, 'g>) -> std::result::Result<Reads, Outcome> {
        Ok(Reads {
            player_writes: host.game.transactions.player_writes(),
        })
    }
    fn view(reads: &Reads) -> &Reads {
        reads
    }
    fn freeze(&self, _: &Reads, plan: Plan) -> std::result::Result<Plan, InvalidReason> {
        Ok(plan)
    }
    fn empty(_: &Plan) -> bool {
        false
    }
    fn validate(&self, host: &GameHost<'a, 'g>, reads: &Reads) -> std::result::Result<(), Outcome> {
        if self.shared.epoch == Epoch::Player
            && host.game.transactions.player_writes() != reads.player_writes
        {
            return Err(Outcome::Stale(StaleReason::Mutation));
        }
        if (self.shared.fresh)(host.game) {
            Ok(())
        } else {
            Err(Outcome::Stale(StaleReason::Snapshot))
        }
    }
    fn apply(&self, host: &mut GameHost<'a, 'g>, _: Plan) {
        let perform = self.shared.perform.take().expect("an action runs once");
        perform(host.game);
    }
}

impl Game<'_> {
    /// Decide an action `name` on the menu `route` for the player entity `actor`: `fire` calls the
    /// hook and validates what it answered (`Reply::Allow`, or `Invalid` to refuse), `fresh` says
    /// the menu is still the one the action froze, and `perform` is the action, run once inside the
    /// savepoint when the decision stands. The result says whether it did.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game) fn decided_action<'p>(
        &mut self,
        name: &'static str,
        route: MenuId,
        actor: EntityRef,
        epoch: Epoch,
        fresh: impl Fn(&Game<'_>) -> bool + 'p,
        fire: impl FnOnce(&mut Game<'_>) -> Reply,
        perform: impl FnOnce(&mut Game<'_>) + 'p,
    ) -> bool {
        let identity = Identity {
            side: Side::Server,
            world: actor.epoch(),
            actor,
            target: actor,
        };
        let target = Action {
            id: route,
            shared: Rc::new(Shared {
                epoch,
                fresh: Box::new(fresh),
                perform: RefCell::new(Some(Box::new(perform))),
            }),
        };
        // The action's savepoint needs a transaction to nest in: the open one, or this one.
        let seal = self.transaction_open();
        let outcome = {
            let mut host = GameHost {
                game: &mut *self,
                identity,
            };
            decision::run(&mut host, &target, |_, _| Plan, |host| fire(host.game))
        };
        self.transaction_close(&seal, matches!(outcome, Ok(Outcome::Applied)));
        match outcome {
            Ok(Outcome::Applied) => true,
            Ok(other) => {
                log::warn!("{name}: the action was refused ({other:?}); nothing changed");
                false
            }
            Err(admission) => {
                log::warn!("{name}: the action was not admitted ({admission:?}); nothing changed");
                false
            }
        }
    }

    /// The player entity of the guest `conn`, as the decision names it.
    pub(in crate::game) fn guest_actor(&self, conn: u32) -> Option<EntityRef> {
        let epoch = self.native_lifecycle.epoch()?;
        let entity = self.lan.as_ref()?.hub.guest(conn)?.entity_id;
        Some(EntityRef::new(epoch, u32::try_from(entity).ok()?))
    }

    /// The decision's menu id for the guest `conn`'s hub menus.
    pub(in crate::game) fn guest_route(&mut self, conn: u32) -> Option<MenuId> {
        Some(self.lan.as_mut()?.menus.entry(conn).or_default().hub.id())
    }
}

impl Game<'_> {
    /// `native_menu_clicked`, the mods told of a click on a menu one of them opened. With a
    /// listener the notice runs in an action: a listener that fails, or whose menu is busy, has its
    /// writes undone. The click moved nothing, so there is nothing to apply and nothing read.
    pub(in crate::game) fn menu_clicked_decided(
        &mut self,
        uuid: u128,
        window: i32,
        slot: i32,
        button: i32,
        mode: i32,
    ) {
        let conn = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().find(|g| g.uuid == uuid))
            .map(|g| g.conn);
        let decided = conn
            .filter(|_| mod_api::__private::listened(mod_api::HookId::MenuClicked))
            .and_then(|conn| Some((self.guest_actor(conn)?, self.guest_route(conn)?)));
        let Some((actor, route)) = decided else {
            return self.native_menu_clicked(uuid, window, slot, button, mode);
        };
        self.decided_action(
            "menu_clicked",
            route,
            actor,
            Epoch::Off,
            |_| true,
            |game| {
                game.native_menu_clicked(uuid, window, slot, button, mode);
                Reply::Allow
            },
            |_| {},
        );
    }
}
