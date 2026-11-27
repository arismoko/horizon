//! Position-owned state follows the payload; observers are invalidated at the owning boundary.
use crate::game::ServerGame;
use sunrise_gameplay_common::game::piston_carry_common::PositionChange;
use sunrise_level::world::BlockEntityExtras;

impl ServerGame<'_> {
    /// Runs before with_level returns: no menu, ticker, save or packet sees an old owner.
    pub(in crate::game::server_side) fn finish_piston_position_changes(
        &mut self,
        changes: Vec<PositionChange>,
    ) {
        if changes.is_empty() {
            return;
        }
        for change in changes {
            match change {
                PositionChange::Carry { from, to, partner } => {
                    for at in [from, to] {
                        let p = [at.0, at.1, at.2];
                        self.server.be_anims.remove_position(p);
                        self.common.world.shulkers.remove(&p);
                        self.common.world.chest_openers.remove(&at);
                        self.server.lid_openers.remove(&at);
                    }
                    self.server
                        .mod_container_viewers
                        .retain(|(_, at), _| *at != from);
                    if self
                        .common
                        .world
                        .block_entities
                        .carried(&to)
                        .map(|c| &c.entity)
                        .or_else(|| self.common.world.block_entities.get(&to))
                        .is_some_and(|e| {
                            matches!(e, sunrise_level::containers::BlockEntity::Jukebox(_))
                        })
                    {
                        self.jukebox_removed([from.0, from.1, from.2]);
                    }
                    if self
                        .common
                        .container
                        .is_some_and(|at| at == from || Some(at) == partner)
                    {
                        self.side_close_container();
                    }
                    let guests: Vec<_> = self
                        .server
                        .lan
                        .iter()
                        .flat_map(|h| {
                            h.hub
                                .guests
                                .simulation_iter()
                                .filter(move |g| {
                                    g.menu.as_ref().is_some_and(|m| match m.slots {
                                        sunrise_sim::lan::menu::Slots::Block(at) => at == from,
                                        sunrise_sim::lan::menu::Slots::Double(a, b) => {
                                            a == from || b == from
                                        }
                                        _ => m.pos == Some(from),
                                    }) || h.menus.get(&g.conn).is_some_and(|m| {
                                        m.station
                                            .as_ref()
                                            .is_some_and(|s| s.pos == [from.0, from.1, from.2])
                                    }) || h.play.get(&g.conn).is_some_and(|p| {
                                        p.lectern.as_ref().is_some_and(|l| l.key == from)
                                    })
                                })
                                .map(|g| g.conn)
                        })
                        .collect();
                    for conn in guests {
                        self.guest_close_menus_and_tell(conn);
                    }
                }
                PositionChange::Land { at } => {
                    if self.common.world.block_entities.get(&at).is_none() {
                        self.common
                            .world
                            .set_block_entity_extras(at, BlockEntityExtras::default());
                        self.common.world.block_names.remove(&at);
                        self.server.hoppers.remove(&at);
                    }
                    self.mark_block_entity_updated(at);
                }
            }
        }
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;
