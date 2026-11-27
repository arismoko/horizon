//! Player candidates for EntityGetter.isUnobstructed while a guest owns the
//! game body. Identity keeps borrowed ancestor bodies distinct from hub copies.
use super::Dim;
use std::cell::Cell;
use std::rc::Rc;
use sunrise_core::geometry::Aabb;

#[derive(Clone, Copy)]
pub(in crate::game) struct BuildingPlayer {
    pub(in crate::game) conn: Option<u32>,
    pub(in crate::game) dimension: Dim,
    pub(in crate::game) bounds: Option<Aabb>,
    pub(in crate::game) borrowed: bool,
}

/// The hub's bodies, shared by the turns of one tick instead of built for each
/// turn. A body is refreshed in place when its guest's turn (or anything that
/// changes it) ends, so every holder sees it where it is now.
pub(in crate::game) type HubBodies = Rc<Vec<Cell<BuildingPlayer>>>;

pub(in crate::game) fn shared(players: impl IntoIterator<Item = BuildingPlayer>) -> HubBodies {
    Rc::new(players.into_iter().map(Cell::new).collect())
}

/// The players' boxes a guest's turn sees: the borrowed ancestor bodies, and
/// the hub's bodies but for the actor.
#[derive(Default)]
pub struct BuildingPlayers {
    ancestors: Vec<BuildingPlayer>,
    actor: Option<u32>,
    hub: HubBodies,
}

impl BuildingPlayers {
    /// Refresh hub-owned bodies, preserve live ancestor bodies held outside
    /// the hub, and omit the actor (GameLevel.player_box supplies that body).
    pub(in crate::game::server_side) fn for_turn(
        &self,
        actor: u32,
        outer: Option<BuildingPlayer>,
        current: HubBodies,
    ) -> Self {
        // The borrowed ancestors are the few nested turns (usually none or
        // one): only they are copied, the hub's bodies are shared.
        let mut ancestors: Vec<_> = self
            .ancestors
            .iter()
            .copied()
            .filter(|p| {
                p.borrowed
                    && p.conn != Some(actor)
                    && (p.conn.is_none() || current.iter().any(|g| g.get().conn == p.conn))
            })
            .collect();
        if let Some(outer) = outer {
            ancestors.retain(|p| p.conn != outer.conn);
            if outer.conn != Some(actor)
                && (outer.conn.is_none() || current.iter().any(|g| g.get().conn == outer.conn))
            {
                ancestors.push(outer);
            }
        }
        Self {
            ancestors,
            actor: Some(actor),
            hub: current,
        }
    }

    pub fn iter(&self, dimension: Dim) -> impl Iterator<Item = Aabb> + '_ {
        // Hub conns are unique, so only the ancestors can already hold one.
        let hub =
            self.hub.iter().map(Cell::get).filter(|p| {
                p.conn != self.actor && !self.ancestors.iter().any(|a| a.conn == p.conn)
            });
        self.ancestors
            .iter()
            .copied()
            .chain(hub)
            .filter(move |p| p.dimension == dimension)
            .filter_map(|p| p.bounds)
    }

    pub fn boxes(&self, dimension: Dim) -> Vec<([f64; 3], [f64; 3])> {
        self.iter(dimension)
            .map(|bounds| {
                (
                    bounds.min_position().components(),
                    bounds.max_position().components(),
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    pub fn player(conn: Option<u32>, x: f64, dimension: Dim, borrowed: bool) -> BuildingPlayer {
        BuildingPlayer {
            conn,
            dimension,
            borrowed,
            bounds: Some(Aabb::new(
                Vec3::new(x, 0.0, 0.0),
                Vec3::new(x + 0.6, 1.8, 0.6),
            )),
        }
    }
    pub fn xs(players: &BuildingPlayers, dim: Dim) -> Vec<f64> {
        players.boxes(dim).iter().map(|(lo, _)| lo[0]).collect()
    }

    use super::*;
    use sunrise_core::geometry::Vec3;

    #[test]
    fn nested_turns_preserve_host_and_live_ancestors_without_the_new_actor() {
        let hub = || {
            vec![
                player(Some(1), 10.0, Dim::Overworld, false),
                player(Some(2), 20.0, Dim::Overworld, false),
                player(Some(3), 30.0, Dim::Nether, false),
            ]
        };
        let host = player(None, 0.0, Dim::Overworld, true);
        let first = BuildingPlayers::default().for_turn(1, Some(host), shared(hub()));
        let second = first.for_turn(
            2,
            Some(player(Some(1), 11.0, Dim::Overworld, true)),
            shared(hub()),
        );
        assert_eq!(xs(&second, Dim::Overworld), [0.0, 11.0]);
        // A further turn in another level keeps each ancestor's own level.
        let third = second.for_turn(
            3,
            Some(player(Some(2), 21.0, Dim::Nether, true)),
            shared(hub()),
        );
        assert_eq!(xs(&third, Dim::Overworld), [0.0, 11.0]);
        assert_eq!(xs(&third, Dim::Nether), [21.0]);
    }

    #[test]
    fn returning_inner_turn_refreshes_moved_and_inactive_hub_bodies() {
        let host = player(None, 0.0, Dim::Overworld, true);
        let first = BuildingPlayers::default().for_turn(
            1,
            Some(host),
            shared(vec![
                player(Some(1), 10.0, Dim::Overworld, false),
                player(Some(2), 20.0, Dim::Overworld, false),
            ]),
        );
        // Restore the first turn after player 2 moved across dimensions.
        let restored = first.for_turn(
            1,
            None,
            shared(vec![
                player(Some(1), 10.0, Dim::Overworld, false),
                player(Some(2), 25.0, Dim::Nether, false),
            ]),
        );
        assert_eq!(xs(&restored, Dim::Overworld), [0.0]);
        assert_eq!(xs(&restored, Dim::Nether), [25.0]);
        // Death, spectator mode, and unloaded bodies provide no building box.
        let mut inactive = player(Some(2), 25.0, Dim::Nether, false);
        inactive.bounds = None;
        let restored = restored.for_turn(1, None, shared(vec![inactive]));
        assert_eq!(xs(&restored, Dim::Overworld), [0.0]);
        assert!(xs(&restored, Dim::Nether).is_empty());
    }

    #[test]
    fn disconnected_ancestors_and_inactive_outer_bodies_do_not_linger() {
        let first = BuildingPlayers::default().for_turn(
            2,
            Some(player(Some(1), 10.0, Dim::Overworld, true)),
            shared(vec![player(Some(1), 1.0, Dim::Overworld, false)]),
        );
        let disconnected = first.for_turn(3, None, shared(Vec::new()));
        assert!(xs(&disconnected, Dim::Overworld).is_empty());
        let disconnected_outer = first.for_turn(
            3,
            Some(player(Some(1), 11.0, Dim::Overworld, true)),
            shared(Vec::new()),
        );
        assert!(xs(&disconnected_outer, Dim::Overworld).is_empty());
        let mut outer = player(None, 0.0, Dim::Overworld, true);
        let first = BuildingPlayers::default().for_turn(1, Some(outer), shared(Vec::new()));
        outer.bounds = None;
        let second = first.for_turn(2, Some(outer), shared(Vec::new()));
        assert!(xs(&second, Dim::Overworld).is_empty());
    }
}
