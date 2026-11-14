//! Resolve a parsed entity selector against the game's loaded levels.
use super::{Game, Selector, Sort, Target};
use horizon_core::random::RandomSource;

impl Game<'_> {
    /// EntitySelector.findEntities.
    pub(in crate::game) fn select(&mut self, s: &Selector) -> Vec<Target> {
        if let Some(name) = &s.player_name {
            return self.player_named(name).into_iter().collect();
        }
        if let Some(uuid) = s.uuid {
            return self.target_by_uuid(uuid).into_iter().collect();
        }
        let origin = {
            let p = self.source_position();
            [
                s.pos[0].unwrap_or(p[0]),
                s.pos[1].unwrap_or(p[1]),
                s.pos[2].unwrap_or(p[2]),
            ]
        };
        let mut source_candidates: Vec<Target> = Vec::new();
        match (s.kind, self.command_source) {
            // @s: the executing entity (none for a server source).
            ('s', Some(source)) => source_candidates.extend(source.entity),
            ('s', None) => source_candidates.extend(self.source_entity()),
            // A guest in place is among the players, in join order.
            _ if self.player.is_some() && self.lan_acting.is_none() => {
                source_candidates.push(Target::Player);
            }
            _ => {}
        }
        if s.kind != 's' {
            source_candidates.extend(self.player_candidates(s));
        }
        let source_dim = self.dim;
        // A selector with a spatial constraint reaches the level the command
        // runs in only; an unbounded one reaches every loaded level
        // (EntitySelector.findEntities).
        let dims = if s.world_limited() {
            vec![source_dim]
        } else {
            self.loaded_dims()
        };
        let mut out: Vec<(Target, f64)> = Vec::new();
        for dim in dims {
            let back = self.enter_command_level(dim);
            let mut candidates = Vec::new();
            if dim == source_dim {
                candidates.append(&mut source_candidates);
            }
            if s.kind != 's' {
                if s.kind == 'e' && matches!(s.sort, Sort::Arbitrary) && !s.world_limited() {
                    candidates.extend(self.global_selector_candidates());
                } else {
                    candidates.extend(self.level_candidates());
                }
            }
            out.extend(self.select_in_level(candidates, s, origin));
            if let Some(back) = back {
                self.enter_command_level(back);
            }
        }
        match s.sort {
            Sort::Nearest => out.sort_by(|a, b| a.1.total_cmp(&b.1)),
            Sort::Furthest => out.sort_by(|a, b| b.1.total_cmp(&a.1)),
            Sort::Random => {
                for i in (1..out.len()).rev() {
                    let j = self.world.random.next_int_bound(i as i32 + 1).unwrap_or(0) as usize;
                    out.swap(i, j);
                }
            }
            Sort::Arbitrary => {}
        }
        out.into_iter()
            .take(s.limit.max(1))
            .map(|(t, _)| t)
            .collect()
    }
}
