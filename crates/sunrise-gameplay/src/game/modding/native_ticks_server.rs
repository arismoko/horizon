//! Server-only entity tick helpers (`others` family, server players, section-scope gate).
use crate::game::ServerGame;
use sunrise_gameplay_common::game::native_ticks::{TickRound, Told};

/// Whether an entity the server keeps without simulating it (`others`) is told its tick here.
pub(in crate::game) fn other_ticks(kind: &str) -> bool {
    !matches!(kind, "minecraft:leash_knot" | "minecraft:eye_of_ender")
        && !super::general::quarantined_kind(kind)
}

impl ServerGame<'_> {
    pub(in crate::game) fn server_tick_uses_server_sections(&self) -> bool {
        self.entity_section_scope().is_some()
    }

    /// The entity the server keeps unsimulated at `others[i]` (`other_ticks`), at its own place
    /// in the join order: its Pre, then its Post, as nothing of it moves in between. One the Pre
    /// canceled or took out of the level hears no Post.
    pub(in crate::game) fn native_other_tick_alone(&mut self, told: Told, i: usize) {
        if !told.listening() {
            return;
        }
        let mut buffers = std::mem::take(&mut self.common.creatures.tick_hooks);
        let (id, join) = (self.server.others.0[i].id, self.server.others.0[i].join);
        buffers.name.clear();
        buffers.name.push_str(&self.server.others.0[i].kind);
        let mut round = TickRound {
            told,
            ..TickRound::default()
        };
        if self.native_tick_pre_one(&mut round, id, &buffers.name)
            && self.server.others.0.locate(join).is_some()
        {
            self.native_tick_post_one(&round, id, &buffers.name);
        }
        buffers.name.clear();
        self.common.creatures.tick_hooks = buffers;
    }

    /// Whether a mod's Pre canceled guest `conn`'s `ServerPlayer.tick` this tick: each of its
    /// parts skips that guest. One branch while no Pre canceled a player.
    pub(in crate::game) fn player_tick_canceled(&self, conn: u32) -> bool {
        let canceled = &self.common.creatures.canceled_players;
        !canceled.is_empty() && canceled.contains(&conn)
    }
}

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use sunrise_gameplay_common::game::native_ticks::NativeTicksGameExt1 as _;
