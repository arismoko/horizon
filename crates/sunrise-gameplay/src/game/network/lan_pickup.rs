//! A click on a LAN guest's Game-owned menu (the inventory, crafting table, machine, station,
//! lectern and mount menus), with the pickup decided by the mods first when any listens to
//! `item_stacked_on_other` (`stacked_pickup`). The trading menu builds its own and does the same
//! in `lan_trading.rs`.
use super::lan_crafting::input;
use sunrise_gameplay_common::game::stacked_on_other::Pickup;
use sunrise_protocol::packets::play;
use sunrise_sim::menu::{Env, Input, Menu};

/// The click a menu op makes: slot, button and input mode.
pub(in crate::game) type Click = (i32, i32, Input);

/// The click a packet makes.
pub(in crate::game) fn click_of(p: &play::ContainerClick) -> Click {
    (i32::from(p.slot), i32::from(p.button), input(p.input))
}

impl crate::game::ServerGame<'_> {
    /// `guest_menu_op`, with `click` (the click `op` makes, if it makes one) decided by the mods
    /// first. With no listener, no click or a click that is not a pickup this is `guest_menu_op`.
    /// A pickup the mods listen to is frozen, decided and then run inside the decision's
    /// savepoint; a refused one changes nothing and gives `None`, as does a guest who is gone.
    pub(in crate::game) fn guest_menu_op_decided<R>(
        &mut self,
        gi: usize,
        menu: &mut Menu,
        click: Option<Click>,
        prepare: Option<&dyn Fn(&mut Menu, &Env)>,
        op: impl FnOnce(&mut Menu, &Env) -> R,
    ) -> Option<R> {
        self.guest_menu_op_settled(gi, menu, click, prepare, op, |_, _, _| {})
    }

    /// `guest_menu_op_decided` for a menu whose click is written back to storage outside the menu
    /// (a machine's block entity, linked containers): `write_back` gets the game, the menu and
    /// what `op` returned, and runs right after `op`, inside the decision's savepoint when there
    /// is one, so an outer abort restores the storage together with the cursor and the slots.
    pub(in crate::game::server_side) fn guest_menu_op_settled<R>(
        &mut self,
        gi: usize,
        menu: &mut Menu,
        click: Option<Click>,
        prepare: Option<&dyn Fn(&mut Menu, &Env)>,
        op: impl FnOnce(&mut Menu, &Env) -> R,
        write_back: impl FnOnce(&mut crate::game::ServerGame<'_>, &mut Menu, &R),
    ) -> Option<R> {
        self.guest_menu_action(gi, menu, click, prepare, |game, menu, stacked| {
            let ran = game.guest_menu_op_with(gi, menu, stacked, op)?;
            write_back(game, menu, &ran);
            Some(ran)
        })
    }

    /// Run an authoritative menu action, deciding a pickup click first when a mod listens.
    /// The action can release the menu's `Env` between takes and continue inside the savepoint.
    pub(in crate::game) fn guest_menu_action<R>(
        &mut self,
        gi: usize,
        menu: &mut Menu,
        click: Option<Click>,
        prepare: Option<&dyn Fn(&mut Menu, &Env)>,
        action: impl FnOnce(
            &mut crate::game::ServerGame<'_>,
            &mut Menu,
            Option<&sunrise_sim::menu::StackedOutcome>,
        ) -> Option<R>,
    ) -> Option<R> {
        let Some((slot, button, mode)) =
            click.filter(|c| matches!(c.2, Input::Pickup | Input::QuickCraft))
        else {
            return action(self, menu, None);
        };
        if !sunrise_gameplay_common::game::stacked_listened() {
            return action(self, menu, None);
        }
        let mut found = None;
        let mut copy = menu.clone();
        let planned = self.guest_menu_op(gi, &mut copy, |menu, env| {
            if let Some(prepare) = prepare {
                prepare(menu, env);
            }
            found = menu.stacked_click(env, slot, button, mode);
        });
        let (Some(frozen), Some(())) = (
            found.filter(sunrise_gameplay_common::game::stacked_listens_for),
            planned,
        ) else {
            return action(self, menu, None);
        };
        let conn = self.server.lan.as_ref()?.hub.guests[gi].conn;
        let route = self
            .server
            .lan
            .as_mut()?
            .menus
            .entry(conn)
            .or_default()
            .hub
            .id();
        let mut ran = None;
        let (carried, same, resolved_slot, primary) = (
            frozen.carried.clone(),
            frozen.clone(),
            frozen.slot,
            frozen.primary,
        );
        let mut probe = copy.clone();
        let pickup = self.stacked_pickup(
            route,
            frozen,
            move |game| {
                let now = game
                    .server
                    .lan
                    .as_ref()?
                    .hub
                    .guests
                    .get(gi)?
                    .carried
                    .clone();
                (now == carried).then(|| same.clone())
            },
            move |game, carried, clicked| {
                game.guest_menu_op(gi, &mut probe, |menu, env| {
                    menu.probe_carried_vanilla_override(
                        env,
                        resolved_slot,
                        primary,
                        carried,
                        clicked,
                    )
                })
                .unwrap_or(false)
            },
            |game, answer| {
                game.journal_guest_click(conn);
                ran = action(game, menu, answer);
            },
        );
        match pickup {
            Pickup::Ran => ran,
            Pickup::Refused => None,
        }
    }

    /// Click `p` on the guest `gi`'s `menu`, after `prepare` has set the menu up as the click finds
    /// it, and say which menu slots the click changed (`None` when the guest is gone or the click
    /// was refused).
    pub(in crate::game::server_side) fn guest_menu_click(
        &mut self,
        gi: usize,
        menu: &mut Menu,
        p: &play::ContainerClick,
        prepare: impl Fn(&mut Menu, &Env),
    ) -> Option<Vec<usize>> {
        let (slot, button, mode) = click_of(p);
        let prepare: &dyn Fn(&mut Menu, &Env) = &prepare;
        self.guest_menu_op_decided(
            gi,
            menu,
            Some((slot, button, mode)),
            Some(prepare),
            |menu, env| {
                prepare(menu, env);
                let before: Vec<_> = (0..menu.slot_count())
                    .map(|s| menu.slot(s).cloned())
                    .collect();
                let _ = menu.click(env, slot, button, mode);
                (0..menu.slot_count())
                    .filter(|&s| menu.slot(s) != before[s].as_ref())
                    .collect()
            },
        )
    }
}

use sunrise_gameplay_common::game::stacked_on_other::StackedOnOtherGameExt1 as _;
