//! One decision for a hook that is not a pickup click (`stacked_on_other.rs` is the pickup's): the
//! mod menu's shift-click (`menu_quick_move`), workstations and trades. The same coordinator, the
//! same savepoint, the same refusal.
//!
//! With no listener the caller makes the ordinary call and nothing here runs. With one, the game
//! opens an action savepoint, fires the hook, checks that nothing the action read was written
//! while the listener ran (`Epoch`), and only then performs the action, inside the same
//! savepoint. A stale read, an invalid answer, a panicking listener and a busy menu leave it
//! undone with every write the listener made rolled back.
use super::*;
use mod_api::EntityRef;
use std::{cell::RefCell, rc::Rc};
use sunrise_gameplay_common::game::transactions::decision::{
    self, ActionTarget, GameHost, Identity, InvalidReason, MenuId, Outcome, Reply, StaleReason,
};

/// What a decision reads of the journals.
pub(in crate::game) enum Epoch {
    /// Players' state (inventory, cursor, menu storage). The epoch is one count for all players,
    /// so a write to any player's state makes the decision stale, not only the acting player's: a
    /// cautious refusal, never a partial move. A write to another block, which a mod's move
    /// legitimately makes, does not make the decision stale.
    Player,
}

/// The decision's menu owner and the lifecycle identity of its actor and target.
pub(in crate::game) struct DecidedTarget {
    pub(in crate::game) route: MenuId,
    pub(in crate::game) identity: Identity,
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// How many times `decided_action_on` was entered: the no-listener tests prove it stays zero.
    pub static DECIDED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

type Fresh<'p, G> = Box<dyn Fn(&G) -> bool + 'p>;
type Perform<'p, G> = Box<dyn FnOnce(&mut G) + 'p>;

struct Shared<'p, S> {
    epoch: Epoch,
    fresh: Fresh<'p, S>,
    perform: RefCell<Option<Perform<'p, S>>>,
}

struct Action<'p, S> {
    id: MenuId,
    shared: Rc<Shared<'p, S>>,
}

impl<S> Clone for Action<'_, S> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            shared: Rc::clone(&self.shared),
        }
    }
}

pub struct Reads {
    player_writes: u64,
}

struct Plan;

impl<'a, 'g, G: sunrise_gameplay_common::game::SideGame<'g>> ActionTarget<GameHost<'a, 'g, G>>
    for Action<'_, G>
{
    type Reads = Reads;
    type View = Reads;
    type Plan = Plan;
    type Frozen = Plan;

    fn id(&self) -> MenuId {
        self.id
    }
    fn capture(&self, host: &GameHost<'a, 'g, G>) -> std::result::Result<Reads, Outcome> {
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
    fn validate(
        &self,
        host: &GameHost<'a, 'g, G>,
        reads: &Reads,
    ) -> std::result::Result<(), Outcome> {
        let Epoch::Player = self.shared.epoch;
        if host.game.transactions.player_writes() != reads.player_writes {
            return Err(Outcome::Stale(StaleReason::Mutation));
        }
        if (self.shared.fresh)(host.game) {
            Ok(())
        } else {
            Err(Outcome::Stale(StaleReason::Snapshot))
        }
    }
    fn apply(&self, host: &mut GameHost<'a, 'g, G>, _: Plan) {
        let perform = self.shared.perform.take().expect("an action runs once");
        perform(host.game);
    }
}

impl crate::game::ServerGame<'_> {
    /// Run a decision whose action target is distinct from its acting player.
    pub(in crate::game) fn decided_action_on<'p>(
        &mut self,
        name: &'static str,
        target: DecidedTarget,
        epoch: Epoch,
        fresh: impl Fn(&crate::game::ServerGame<'_>) -> bool + 'p,
        fire: impl FnOnce(&mut crate::game::ServerGame<'_>) -> Reply,
        perform: impl FnOnce(&mut crate::game::ServerGame<'_>) + 'p,
    ) -> bool {
        let DecidedTarget { route, identity } = target;
        #[cfg(any(test, feature = "test-support"))]
        DECIDED.set(DECIDED.get() + 1);
        let target = Action {
            id: route,
            shared: Rc::new(Shared {
                epoch,
                fresh: Box::new(fresh),
                perform: RefCell::new(Some(Box::new(perform))),
            }),
        };
        // The action's savepoint needs a transaction to nest in: the open one, or this one.
        let outcome = self.storage_scope(|game, _| {
            let mut host = GameHost::new(game, identity);
            let outcome = decision::run(&mut host, &target, |_, _| Plan, |host| fire(host.game));
            let commit = matches!(outcome, Ok(Outcome::Applied));
            (outcome, commit)
        });
        match outcome {
            Ok(Outcome::Applied) => true,
            // A cancelled action is the listener's own answer, not a refusal: the caller goes on.
            Ok(Outcome::Canceled) => false,
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
        let epoch = self.common.native_lifecycle.epoch()?;
        let entity = self.server.lan.as_ref()?.hub.guest(conn)?.entity_id;
        Some(EntityRef::new(epoch, u32::try_from(entity).ok()?))
    }

    /// Keep guest `conn`'s inventory, armor, off hand, equipment and cursor (and their crafting
    /// grid, ender chest and mod menu storage), as they are, in the open transaction.
    pub(in crate::game) fn journal_guest_click(&mut self, conn: u32) {
        let Some(uuid) = self
            .server
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

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
