//! Client local and creative menu pickup route for `item_stacked_on_other`.
use super::*;
use std::rc::Rc;
use sunrise_gameplay_common::game::stacked_on_other::{
    Pickup, local_pickup_id, stacked_listened_for_side, stacked_listens_for_side,
};
use sunrise_sim::menu::{Env, Input};

impl crate::game::ClientGame<'_> {
    /// The clicks `menu_ops` was asked for, when they resolve to a pickup of a slot of the local
    /// or creative menu.
    pub(in crate::game::client_side) fn stacked_menu_ops(
        &mut self,
        ops: &[(Option<usize>, i32, Input)],
    ) -> Option<bool> {
        if self.client.remote.is_some() && self.client.screen != Screen::Creative {
            return None;
        }
        let server = self.common.is_logical_server();
        if !stacked_listened_for_side(server) || ops.is_empty() {
            return None;
        }
        let is_single_pickup = matches!(ops, [(Some(slot), _, Input::Pickup)] if *slot != 200);
        let is_quick_craft = ops.iter().all(|&(_, _, input)| input == Input::QuickCraft)
            && !ops.iter().any(|&(slot, _, _)| slot == Some(200));
        if !is_single_pickup && !is_quick_craft {
            return None;
        }
        self.stacked_menu_gesture(ops)
    }

    fn stacked_menu_gesture(&mut self, ops: &[(Option<usize>, i32, Input)]) -> Option<bool> {
        let server = self.common.is_logical_server();
        let mut found = None;
        let mut mapped = Vec::new();
        let modeled = self.menu_session(|menu, env, to_menu| {
            let mut clicks = Vec::with_capacity(ops.len());
            for &(slot, button, input) in ops {
                let menu_slot = match slot {
                    Some(g) => {
                        if g == 200 && input != Input::QuickMove {
                            return None;
                        }
                        to_menu(g)? as i32
                    }
                    None => -999,
                };
                clicks.push((menu_slot, button, input));
            }
            let (last, prefix) = clicks.split_last()?;
            for &(slot, button, input) in prefix {
                if menu.click(env, slot, button, input).is_err() {
                    return Some(false);
                }
            }
            found = menu.stacked_click(env, last.0, last.1, last.2);
            mapped = clicks;
            Some(false)
        });
        let frozen = found
            .filter(|_| modeled)
            .filter(|frozen| stacked_listens_for_side(server, frozen))?;
        let handled = Rc::new(std::cell::Cell::new(false));
        let result = handled.clone();
        let (carried, resolved_slot, primary) =
            (frozen.carried.clone(), frozen.slot, frozen.primary);
        let clicks = mapped;
        let pickup = self.stacked_pickup(
            local_pickup_id(),
            frozen.clone(),
            move |game| (game.carried == carried).then(|| frozen.clone()),
            |game, carried, clicked| {
                let mut took = false;
                let _ = game.menu_session(|menu, env, _| {
                    if let Some((_, prefix)) = clicks.split_last() {
                        for &(slot, button, input) in prefix {
                            if menu.click(env, slot, button, input).is_err() {
                                return Some(false);
                            }
                        }
                    }
                    took = menu.probe_carried_vanilla_override(
                        env,
                        resolved_slot,
                        primary,
                        carried,
                        clicked,
                    );
                    Some(false)
                });
                took
            },
            |game, answer| {
                let player = game.native_acting_id();
                game.journal_player(player);
                if let Some(key) = game.container {
                    game.journal_block_entity(key);
                }
                result.set(game.menu_session(|menu, env, _| {
                    let Some((last, prefix)) = clicks.split_last() else {
                        return Some(false);
                    };
                    for &(slot, button, input) in prefix {
                        if menu.click(env, slot, button, input).is_err() {
                            return Some(false);
                        }
                    }
                    let env = Env {
                        stacked: answer,
                        ..*env
                    };
                    menu.click(&env, last.0, last.1, last.2)
                        .map(|()| true)
                        .ok()
                        .or(Some(false))
                }));
            },
        );
        Some(pickup == Pickup::Refused || handled.get())
    }
}

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::stacked_on_other::StackedOnOtherGameExt1 as _;
