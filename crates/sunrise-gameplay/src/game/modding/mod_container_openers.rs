//! The server's ContainerOpenersCounter for declared mod containers.
use crate::game::ServerGame;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_sim::mobs::{Species, Who};

type Pos = (i32, i32, i32);

pub struct ModContainerOpeners {
    pub count: u32,
    generation: u64,
    recheck: i64,
    range: f64,
}

impl ServerGame<'_> {
    /// startOpen/stopOpen, with the opening player's body in place.
    pub(in crate::game::server_side) fn mod_container_player_opener(
        &mut self,
        pos: Pos,
        open: bool,
    ) {
        self.mod_container_player_opener_for(pos, open, None);
    }

    pub(in crate::game::server_side) fn mod_container_player_opener_for(
        &mut self,
        pos: Pos,
        open: bool,
        bound: Option<u64>,
    ) {
        if self.common.spectator() {
            return;
        }
        let range = self
            .common
            .interaction_range("minecraft:block_interaction_range", 4.5, 5.0);
        self.change_mod_container_openers(pos, open, Some(Who::Player), range, bound);
    }

    pub(in crate::game::server_side) fn mod_container_opener(
        &mut self,
        pos: Pos,
        open: bool,
        source: Option<Who>,
        range: f64,
    ) {
        self.change_mod_container_openers(pos, open, source, range, None);
    }

    fn change_mod_container_openers(
        &mut self,
        pos: Pos,
        open: bool,
        source: Option<Who>,
        range: f64,
        bound: Option<u64>,
    ) {
        let Some(generation) = self.mod_container_generation(pos) else {
            return;
        };
        if bound.is_some_and(|opened| opened != generation) {
            return;
        }
        let key = (self.common.dim.id(), pos);
        if self
            .server
            .mod_container_viewers
            .get(&key)
            .is_some_and(|c| c.generation != generation)
        {
            self.server.mod_container_viewers.remove(&key);
        }
        let counter = if open {
            let counter =
                self.server
                    .mod_container_viewers
                    .entry(key)
                    .or_insert(ModContainerOpeners {
                        count: 0,
                        generation,
                        recheck: self.common.world.game_time + 5,
                        range,
                    });
            counter.range = counter.range.max(range);
            counter
        } else if let Some(counter) = self.server.mod_container_viewers.get_mut(&key) {
            counter
        } else {
            return;
        };
        let previous = counter.count;
        let count = if open {
            previous.saturating_add(1)
        } else {
            previous.saturating_sub(1)
        };
        counter.count = count;
        self.mod_container_opener_notice(pos, previous, count, source);
    }

    fn mod_container_generation(&self, pos: Pos) -> Option<u64> {
        self.common.mod_menu_at(pos)?;
        self.common.world.block_entities.generation(&pos)
    }

    fn mod_container_opener_notice(
        &mut self,
        pos: Pos,
        previous: u32,
        count: u32,
        source: Option<Who>,
    ) {
        let at = [pos.0, pos.1, pos.2];
        if previous == 0 && count > 0 {
            self.post_block_game_event("minecraft:container_open", at, source, None);
        } else if previous > 0 && count == 0 {
            self.post_block_game_event("minecraft:container_close", at, source, None);
        }
        self.native_mod_container_viewers(self.common.dim.id(), at, count, previous);
    }

    /// Scheduled rechecks only: no player or mob walk without an open container.
    pub fn tick_mod_container_viewers(&mut self) {
        let dim = self.common.dim.id();
        let time = self.common.world.game_time;
        let due: Vec<_> = self
            .server
            .mod_container_viewers
            .iter()
            .filter(|((d, _), c)| *d == dim && c.recheck <= time)
            .map(|((_, pos), c)| (*pos, c.generation))
            .collect();
        for (pos, generation) in due {
            let key = (dim, pos);
            // A scheduled recheck belongs to the block entity that opened, not its replacement.
            // An earlier recheck callback may also have already changed this counter.
            let Some(counter) = self
                .server
                .mod_container_viewers
                .get(&key)
                .filter(|c| c.generation == generation && c.recheck <= time)
            else {
                continue;
            };
            if self.mod_container_generation(pos) != Some(generation) {
                self.server.mod_container_viewers.remove(&key);
                continue;
            }
            let previous = counter.count;
            let range = counter.range + 4.0;
            let area = Aabb::new(
                Vec3::new(
                    pos.0 as f64 - range,
                    pos.1 as f64 - range,
                    pos.2 as f64 - range,
                ),
                Vec3::new(
                    pos.0 as f64 + 1.0 + range,
                    pos.1 as f64 + 1.0 + range,
                    pos.2 as f64 + 1.0 + range,
                ),
            );
            let mut count = 0;
            let mut next_range: f64 = 0.0;
            if let Some(host) = &mut self.server.lan {
                let menus = &host.menus;
                host.hub.guests.update_all(|g| {
                    if g.placed_in(dim)
                        && !g.spectator()
                        && g.bounds().intersects(area)
                        && menus
                            .get(&g.conn)
                            .is_some_and(|m| m.menu_holds(g.menu.as_ref(), pos, generation))
                    {
                        count += 1;
                        let fallback = if g.creative() { 5.0 } else { 4.5 };
                        let reach = g
                            .motion
                            .as_mut()
                            .and_then(|m| m.attribute("minecraft:block_interaction_range").ok())
                            .unwrap_or(fallback);
                        next_range = next_range.max(reach);
                    }
                });
            }
            if self.common.container == Some(pos)
                && !self.common.spectator()
                && self
                    .player_view()
                    .is_some_and(|p| p.bounds.intersects(area))
            {
                count += 1;
                next_range = next_range.max(self.common.interaction_range(
                    "minecraft:block_interaction_range",
                    4.5,
                    5.0,
                ));
            }
            let golems = self
                .common
                .mobs
                .iter()
                .filter(|m| {
                    m.species() == Species::CopperGolem
                        && !m.removed
                        && m.extra.opened_chest == Some([pos.0, pos.1, pos.2])
                        && m.bounds().intersects(area)
                })
                .count() as u32;
            count += golems;
            if golems > 0 {
                next_range = next_range.max(3.0);
            }
            if count == 0 {
                self.server.mod_container_viewers.remove(&key);
            } else if let Some(counter) = self.server.mod_container_viewers.get_mut(&key) {
                counter.count = count;
                counter.range = next_range;
                counter.recheck = time + 5;
            }
            // recheckOpeners uses no entity source, even on a zero crossing.
            self.mod_container_opener_notice(pos, previous, count, None);
        }
    }
}
