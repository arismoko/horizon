//! Role-neutral block-entity animation events, ownership tracking, and state reads.
use sunrise_core::nbt::Tag;
use sunrise_level::block_entity_ticks::EndGateway;
use sunrise_level::{block_entities::Pos, containers::BlockEntity};
use sunrise_sim::save::number;

/// The handle of the local player among a bell's gathered entities.
pub const PLAYER: u64 = u64::MAX;

impl crate::game::CommonGame<'_> {
    /// End gateway at `pos`: live `Age`, saved update tag, or a fresh gateway.
    pub fn gateway_loaded(&self, pos: [i32; 3]) -> EndGateway {
        let live = match self.world.block_entities.get(&(pos[0], pos[1], pos[2])) {
            Some(BlockEntity::EndGateway(g)) => Some(g.age),
            _ => None,
        };
        let age = live.or_else(|| self.saved_tag(pos).and_then(|t| number(t.get("Age"))));
        EndGateway {
            age: age.unwrap_or(0),
            teleport_cooldown: 0,
        }
    }

    /// Saved block entity NBT tag at `pos`, if present on the loaded chunk.
    pub fn saved_tag(&self, pos: [i32; 3]) -> Option<&Tag> {
        self.world
            .chunks
            .get(&(pos[0] >> 4, pos[2] >> 4))
            .and_then(|c| c.saved.as_deref())
            .and_then(|saved| {
                saved.block_entities.iter().find(|t| {
                    [t.get("x"), t.get("y"), t.get("z")]
                        .map(number)
                        .eq(&pos.map(|v| Some(i64::from(v))))
                })
            })
    }

    /// `PotentSulfurBlock.triggerEvent`: stamp the eruption tick on a potent sulfur block entity.
    pub fn potent_sulfur_eruption_event(&mut self, pos: Pos) -> bool {
        let time = self.world.game_time;
        match self.world.block_entities.get_mut(&pos) {
            Some(BlockEntity::PotentSulfur(sulfur)) => {
                sulfur.eruption_tick = time;
                true
            }
            _ => false,
        }
    }
}

/// The sherds (item ids: back, left, right, front) in the saved tag of a decorated pot.
pub fn pot_sherds(saved: Option<&Tag>) -> [Option<String>; 4] {
    let sherds = saved.and_then(|t| t.get("sherds"));
    ["back", "left", "right", "front"].map(|side| match sherds.and_then(|s| s.get(side)) {
        // An item id, or the item stack the processed templates hold.
        Some(Tag::String(id)) => Some(id.to_string_lossy()),
        Some(stack) => sunrise_sim::save::item_from_nbt(stack).map(|s| s.item.to_string()),
        None => None,
    })
}
