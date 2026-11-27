use sunrise_sim::menu::{Menu as EngineMenu, MutationStamp, Snapshot};
use {
    crate::game::ServerGame, sunrise_gameplay_common::game::transactions::decision::ActionTarget,
    sunrise_gameplay_common::game::transactions::decision::Admission,
    sunrise_gameplay_common::game::transactions::decision::GameHost,
    sunrise_gameplay_common::game::transactions::decision::InvalidReason,
    sunrise_gameplay_common::game::transactions::decision::Journal,
    sunrise_gameplay_common::game::transactions::decision::MenuId,
    sunrise_gameplay_common::game::transactions::decision::Outcome,
    sunrise_gameplay_common::game::transactions::decision::OwnerStamp,
    sunrise_gameplay_common::game::transactions::decision::StaleReason,
    sunrise_gameplay_common::game::transactions::decision::engine::KeptOwner,
    sunrise_gameplay_common::game::transactions::decision::engine::Target,
};

#[derive(Clone, Copy)]
pub struct ServerDecisionTarget(Target);

pub struct EngineReads {
    owner: OwnerStamp,
    value: MutationStamp,
    menu: EngineMenu,
}

/// The lower menu planner supplies already frozen values. None is no action,
/// which consumes the decision without dispatch. Effects remain in After.
pub struct EnginePlan(pub Option<Snapshot>);

impl ServerDecisionTarget {
    /// Check the active action before looking in a slot that an outer action
    /// might have lifted. A missing menu is never the reentrancy mechanism.
    pub fn admit(game: &ServerGame<'_>, conn: u32) -> Result<Self, Admission> {
        if game.transactions.iter().any(|level| {
            level
                .action
                .as_ref()
                .and_then(|action| level.actions.owners.get(&action.menu))
                .is_some_and(|kept| kept.target.conn == conn)
        }) {
            return Err(Admission::Busy);
        }
        let menus = game
            .server
            .lan
            .as_ref()
            .and_then(|host| host.menus.get(&conn))
            .ok_or(Admission::MissingHolder)?;
        Ok(Self(Target {
            conn,
            id: menus.crafting.inventory.id(),
        }))
    }

    pub fn owner_id(&self) -> MenuId {
        self.0.owner_id()
    }

    pub fn read<'a>(&self, game: &'a ServerGame<'_>) -> Option<(OwnerStamp, &'a EngineMenu)> {
        let menus = game.server.lan.as_ref()?.menus.get(&self.0.conn)?;
        let inventory = &menus.crafting.inventory;
        Some((inventory.stamp(), inventory.as_deref()?))
    }

    /// Every route must use this boundary before taking or mutating its owner.
    /// First-write snapshots merge into the existing Host transaction journal.
    pub fn edit<R>(
        &self,
        game: &mut ServerGame<'_>,
        edit: impl FnOnce(&mut EngineMenu) -> R,
    ) -> Option<R> {
        if self.read(game)?.0.id != self.0.id {
            return None;
        }
        self.keep_in(game);
        let menus = game.server.lan.as_mut()?.menus.get_mut(&self.0.conn)?;
        let menu = menus.crafting.inventory.as_deref_mut()?;
        Some(edit(menu))
    }

    fn keep_in(&self, game: &mut ServerGame<'_>) {
        let Some(level) = game.transactions.last() else {
            return;
        };
        if level.actions.owners.contains_key(&self.0.id) {
            return;
        }
        let Some(menus) = game
            .server
            .lan
            .as_ref()
            .and_then(|h| h.menus.get(&self.0.conn))
        else {
            return;
        };
        let value = (*menus.crafting.inventory).clone();
        game.transactions.last_mut().unwrap().actions.owners.insert(
            self.0.id,
            KeptOwner {
                target: self.0,
                value,
            },
        );
    }
}

fn rollback_kept_owner(kept: KeptOwner, game: &mut ServerGame<'_>) {
    let Some(menus) = game
        .server
        .lan
        .as_mut()
        .and_then(|h| h.menus.get_mut(&kept.target.conn))
    else {
        return;
    };
    menus.crafting.inventory.restore(kept.value);
}

pub(in crate::game) fn rollback_in(mut journal: Journal, game: &mut ServerGame<'_>) {
    for kept in journal.take_owners() {
        rollback_kept_owner(kept, game);
    }
    journal.rollback();
}

impl<'a, 'g> ActionTarget<GameHost<'a, 'g, ServerGame<'g>>> for ServerDecisionTarget {
    type Reads = EngineReads;
    type View = EngineMenu;
    type Plan = EnginePlan;
    type Frozen = EnginePlan;

    fn id(&self) -> MenuId {
        self.owner_id()
    }

    fn keep(&self, host: &mut GameHost<'a, 'g, ServerGame<'g>>) {
        self.keep_in(host.game);
    }

    fn capture(&self, host: &GameHost<'a, 'g, ServerGame<'g>>) -> Result<EngineReads, Outcome> {
        let (owner, menu) = self
            .read(host.game)
            .ok_or(Outcome::Stale(StaleReason::MenuInstance))?;
        if owner.id != self.0.id {
            return Err(Outcome::Stale(StaleReason::MenuInstance));
        }
        Ok(EngineReads {
            owner,
            value: menu.mutation_stamp(),
            menu: menu.clone(),
        })
    }

    fn view(reads: &EngineReads) -> &EngineMenu {
        &reads.menu
    }

    fn freeze(&self, _reads: &EngineReads, plan: EnginePlan) -> Result<EnginePlan, InvalidReason> {
        Ok(plan)
    }

    fn empty(plan: &EnginePlan) -> bool {
        plan.0.is_none()
    }

    fn validate(
        &self,
        host: &GameHost<'a, 'g, ServerGame<'g>>,
        reads: &EngineReads,
    ) -> Result<(), Outcome> {
        let (owner, menu) = self
            .read(host.game)
            .ok_or(Outcome::Stale(StaleReason::MenuInstance))?;
        if owner != reads.owner {
            return Err(Outcome::Stale(StaleReason::MenuInstance));
        }
        if menu.mutation_stamp() != reads.value {
            return Err(Outcome::Stale(StaleReason::Mutation));
        }
        Ok(())
    }

    fn apply(&self, host: &mut GameHost<'a, 'g, ServerGame<'g>>, plan: EnginePlan) {
        if let Some(snapshot) = plan.0 {
            self.edit(host.game, |menu| menu.restore(snapshot))
                .expect("validated live menu owner");
        }
    }
}
