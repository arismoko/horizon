//! A hub container click the game owns. `Hub::container_click` used to run inside
//! `self.lan.as_mut()`, so nothing it called could reach the game: a hook fired in the middle
//! of a click had no `&Game` to be given. Here the game lifts the host out of itself for the
//! click, so the click code and the game are separate values and the game is whole while the
//! hub runs.
use super::*;
use horizon_client::menu::{PlanningRules, RuleAnswers, StackedOutcome, StoredRules};
use std::{cell::RefCell, rc::Rc};

impl Game<'_> {
    /// Run `f` with the LAN host taken out of `self.lan`, then put it back. `f` is given the whole
    /// game (`self.lan` is `None` while it runs) and the host. The host is restored on every
    /// path, including a panic in `f`, which is then propagated: no borrow of the host outlives
    /// the call and the game is not left without it. `None` when there is no host to lift,
    /// which includes a call made while the host is already lifted.
    pub(in crate::game) fn with_lan_lifted<R>(
        &mut self,
        f: impl FnOnce(&mut Self, &mut LanHost) -> R,
    ) -> Option<R> {
        let mut host = self.lan.take()?;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self, &mut host)));
        debug_assert!(self.lan.is_none(), "a lifted LAN host was replaced");
        self.lan = Some(host);
        match result {
            Ok(result) => Some(result),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// `ServerGamePacketListenerImpl.handleContainerClick` for the guest `conn`. The hub reads
    /// what the click may do and the container it clicks; a pickup the mods listen to is then
    /// decided with the game whole (`stacked_pickup`: they are fired with `HostSide`, the LAN host
    /// still in place); and the click itself runs with the host lifted out of the game. Returns
    /// the stacks the click threw out.
    pub(super) fn lan_hub_click(&mut self, conn: u32, p: &play::ContainerClick) -> Vec<ItemStack> {
        let Some(host) = self.lan.as_mut() else {
            return Vec::new();
        };
        let Some(mut prepared) = host.hub.prepare_click(conn, p, &self.world) else {
            return Vec::new();
        };
        let frozen = if Self::stacked_listened() {
            host.hub.stacked_click(&mut prepared, p)
        } else {
            None
        };
        let route = host.menus.entry(conn).or_default().hub.id();
        // The block entities behind the container the click writes into.
        let blocks: Vec<_> = host
            .hub
            .guest(conn)
            .and_then(|g| g.menu.as_ref())
            .map(|m| match m.slots {
                Slots::Block(key) => vec![key],
                Slots::Double(first, second) => vec![first, second],
                _ => Vec::new(),
            })
            .unwrap_or_default();
        // Plan: ask the slot rules for what the click would consult, once for each distinct
        // question and for each way the hook can leave it, on state put back afterwards.
        let answers = Rc::new(RuleAnswers::default());
        if frozen.is_some() {
            self.with_lan_lifted(|game, host| {
                let planning = PlanningRules {
                    rules: Some(&*game),
                    answers: &answers,
                };
                for outcome in [
                    StackedOutcome::default(),
                    StackedOutcome {
                        canceled: true,
                        ..StackedOutcome::default()
                    },
                ] {
                    let rules = ItemRules(
                        world_items!(game),
                        &world_stations!(game).enchantments,
                        Some(&planning),
                        Some(&outcome),
                    );
                    host.hub.plan_click(&mut prepared, p, &rules);
                }
            });
        }
        let prepared = Rc::new(RefCell::new(Some(prepared)));
        let drops = Rc::new(RefCell::new(Vec::new()));
        if let Some(frozen) = frozen {
            let (shared, thrown, packet) = (prepared.clone(), drops.clone(), p.clone());
            let stored = answers.clone();
            let (stacks, slot) = (frozen.clone(), frozen.slot);
            self.stacked_pickup(
                route,
                frozen,
                move |game| {
                    let (carried, clicked) =
                        game.lan
                            .as_ref()?
                            .hub
                            .click_stacks(conn, slot, &game.world)?;
                    (carried == stacks.carried && clicked == stacks.clicked).then(|| stacks.clone())
                },
                move |game, answer| {
                    // What the click writes belongs to the action: the guest's inventory and
                    // cursor, and the container's block entities.
                    game.journal_guest_click(conn);
                    for key in blocks {
                        game.journal_block_entity(key);
                    }
                    game.hub_click_run(&packet, &shared, &thrown, answer, Some(&*stored))
                },
            );
        } else {
            self.hub_click_run(p, &prepared, &drops, None, None);
        }
        // A click the decision refused never ran: the container, the drag and the client are put
        // back as they were.
        if let Some(prepared) = prepared.take()
            && let Some(host) = self.lan.as_mut()
        {
            host.hub
                .finish_click(prepared, p, &mut self.world, (Vec::new(), Vec::new()));
        }
        drops.take()
    }

    /// Run the prepared click with the LAN host lifted out of the game, the game lent to it as
    /// the mods' `menu_slot_rules` (their read-only `Host`; what it reads of the LAN host answers
    /// as if none were open) and the mods' answer to the pickup, if they gave one.
    fn hub_click_run(
        &mut self,
        p: &play::ContainerClick,
        prepared: &RefCell<Option<horizon_client::lan::containers::PreparedClick>>,
        drops: &RefCell<Vec<ItemStack>>,
        answer: Option<&horizon_client::menu::StackedOutcome>,
        stored: Option<&RuleAnswers>,
    ) {
        let Some(mut click) = prepared.take() else {
            return;
        };
        let thrown = self.with_lan_lifted(|game, host| {
            // Decided clicks use only the answers their plan kept; plain ones ask the game.
            let stored = stored.map(StoredRules);
            let rules = ItemRules(
                world_items!(game),
                &world_stations!(game).enchantments,
                match &stored {
                    Some(stored) => Some(stored),
                    None => Some(&*game),
                },
                answer,
            );
            let outcome = host.hub.run_click(&mut click, p, &rules);
            host.hub.finish_click(click, p, &mut game.world, outcome)
        });
        drops.borrow_mut().extend(thrown.unwrap_or_default());
    }
}
