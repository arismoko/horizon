//! Action state belongs to the existing storage transaction, not a parallel
//! transaction stack. The resource-free harness uses these same journal methods.
use super::*;
use sunrise_sim::join_seq::visibility::PassLease;

/// A marker lives exactly as long as its transaction level. The existing save
/// admission gate checks passes_settled(), including this lease. In particular
/// commands.save.failed is still produced by the existing save-all command.
pub struct ActiveAction {
    pub menu: MenuId,
    _save_gate: PassLease,
}

impl ActiveAction {
    pub(super) fn new(menu: MenuId) -> Self {
        Self {
            menu,
            _save_gate: PassLease::enter(),
        }
    }
}

struct KeptMenu {
    menu: Menu,
    state: MenuState,
}

#[derive(Default)]
pub struct Journal {
    menus: HashMap<MenuId, KeptMenu>,
    pub owners: HashMap<MenuId, engine::KeptOwner>,
    // The harness has no Game::After queue. Production uses that existing queue
    // directly through GameHost::defer, preserving listener/apply effect order.
    #[cfg(any(test, feature = "test-support"))]
    after: Vec<Effect>,
}

impl Journal {
    pub(super) fn keep(&mut self, menu: &Menu) {
        self.menus.entry(menu.id).or_insert_with(|| KeptMenu {
            menu: menu.clone(),
            state: menu.read(Clone::clone),
        });
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn defer(&mut self, effect: Effect) {
        self.after.push(effect);
    }

    /// Keep the oldest snapshot when a nested listener or action commits.
    pub fn merge(&mut self, child: Self) {
        for (id, kept) in child.menus {
            self.menus.entry(id).or_insert(kept);
        }
        for (id, kept) in child.owners {
            self.owners.entry(id).or_insert(kept);
        }
        #[cfg(any(test, feature = "test-support"))]
        self.after.extend(child.after);
    }

    pub fn rollback(self) {
        assert!(self.owners.is_empty(), "game menu undo needs its owner");
        // No success effect, drop, or retirement escapes an aborted action.
        #[cfg(any(test, feature = "test-support"))]
        drop(self.after);
        for kept in self.menus.into_values() {
            let mut inner = kept.menu.inner.borrow_mut();
            inner.epoch.advance();
            inner.state = kept.state;
        }
    }

    pub fn take_owners(&mut self) -> impl Iterator<Item = engine::KeptOwner> {
        std::mem::take(&mut self.owners).into_values()
    }

    /// Only the root commit owner may take the queued effects.
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn root_effects(self) -> Vec<Effect> {
        self.after
    }
}

/// This small internal trait adapts the existing transaction owner. It is not
/// a Host service or a second public transaction API. Stage 3 supplies route
/// effects through the existing After variants and root publisher.
pub trait ActionHost {
    fn identity(&self) -> Identity;
    fn residence(&self, holder: EntityRef) -> Residence;
    fn has_transaction(&self) -> bool;
    fn menu_busy(&self, menu: MenuId) -> bool;
    fn open_action(&mut self, menu: MenuId) -> TransactionFrame;
    fn action_is_current(&self, savepoint: TransactionFrame, menu: MenuId) -> bool;
    fn close_action(&mut self, savepoint: TransactionFrame, commit: bool);
    fn journal(&mut self) -> &mut Journal;
    fn defer(&mut self, effect: Effect);
    fn decided(&mut self) {}
    fn dispatched(&mut self) {}
    fn applied(&mut self) {}
}

/// Stage-2 adapter. No real menu route constructs this yet. Identity is supplied
/// by the action's game-side caller and checked against the live lifecycle on
/// each validation, not recovered from mod-controlled output.
pub struct GameHost<'a, 'g, S: crate::game::SideGame<'g>> {
    pub game: &'a mut S,
    pub identity: Identity,
    resource: std::marker::PhantomData<fn(&'g ()) -> &'g ()>,
}

impl<'a, 'g, S: crate::game::SideGame<'g>> GameHost<'a, 'g, S> {
    #[inline(always)]
    pub fn new(game: &'a mut S, identity: Identity) -> Self {
        Self {
            game,
            identity,
            resource: std::marker::PhantomData,
        }
    }
}

impl<'g, S: crate::game::SideGame<'g>> ActionHost for GameHost<'_, 'g, S> {
    fn identity(&self) -> Identity {
        let mut identity = self.identity;
        if let Some(world) = self.game.native_lifecycle.epoch() {
            identity.world = world;
        }
        identity.side = if self.game.is_logical_server() {
            Side::Server
        } else {
            Side::Client
        };
        identity
    }

    fn residence(&self, holder: EntityRef) -> Residence {
        if self.game.native_lifecycle.epoch() != Some(holder.epoch()) {
            return Residence::Missing;
        }
        // The body in place is the acting player's, whoever it is: it is here by being in place.
        if self
            .game
            .player
            .as_ref()
            .is_some_and(|body| u32::try_from(body.entity_id) == Ok(holder.id()))
        {
            return Residence::Found;
        }
        self.game.side_entity_residence(holder)
    }

    fn has_transaction(&self) -> bool {
        !self.game.transactions.is_empty()
    }

    fn menu_busy(&self, menu: MenuId) -> bool {
        self.game
            .transactions
            .iter()
            .any(|level| level.action.as_ref().is_some_and(|a| a.menu == menu))
    }

    fn open_action(&mut self, menu: MenuId) -> TransactionFrame {
        let frame = self.game.open_storage_transaction();
        self.game.transactions.last_mut().unwrap().action = Some(ActiveAction::new(menu));
        frame
    }

    fn action_is_current(&self, savepoint: TransactionFrame, menu: MenuId) -> bool {
        self.game.transactions.last().is_some_and(|level| {
            level.frame == Some(savepoint) && level.action.as_ref().is_some_and(|a| a.menu == menu)
        })
    }

    fn close_action(&mut self, savepoint: TransactionFrame, commit: bool) {
        let _ = self.game.close_storage_frame(savepoint, commit);
    }

    fn journal(&mut self) -> &mut Journal {
        &mut self
            .game
            .transactions
            .last_mut()
            .expect("write needs transaction")
            .actions
    }

    fn defer(&mut self, effect: Effect) {
        self.game
            .transactions
            .last_mut()
            .expect("effect needs transaction")
            .after
            .push(effect);
    }
}

use crate::game::side_ops::modapi::ModapiSideGameExt as _;

use crate::game::transactions::TransactionsGameExt as _;
