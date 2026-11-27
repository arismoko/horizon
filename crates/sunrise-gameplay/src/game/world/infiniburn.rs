//! Fire sources are resolved when a World receives its dimension facts or new tags.
use super::*;

impl crate::game::ServerGame<'_> {
    pub fn refresh_infiniburn(&mut self) {
        let golem_heads = self.resolve_golem_heads();
        self.common.world.golem_heads = Some(golem_heads.clone());
        self.common.world.infiniburn = Some(self.resolve_level_infiniburn(self.dimension_type()));
        let parked: Vec<_> = self
            .server
            .levels
            .0
            .parked
            .keys()
            .copied()
            .map(|dim| {
                (
                    dim,
                    self.resolve_level_infiniburn(self.common.dimension_type_of(dim)),
                )
            })
            .collect();
        for (dim, set) in parked {
            let world = &mut self.server.levels.0.parked.get_mut(&dim).unwrap().world;
            world.infiniburn = Some(set);
            world.golem_heads = Some(golem_heads.clone());
        }
    }

    /// An opened world's level becomes current: its `World` was installed after the tables
    /// were built, so its fire sources resolve now, against the tags just installed.
    pub(in crate::game::server_side) fn enter_opened_dim(&mut self, dim: Dim) {
        self.common.dim = dim;
        self.refresh_infiniburn();
    }
}
