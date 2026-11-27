//! Give a completed bundle's group data to the packet that owns it and check
//! the net cost against the connection's resident group limit before adoption.
use sunrise_level::{chunk::Chunk, fast_hash::FxHashMap, world::World};
use sunrise_protocol::{
    Error, Result, connection_groups::Section, packets::play::Clientbound as C,
};

#[derive(Clone, Copy)]
pub struct ReceivedGroup(pub Option<u64>);

impl ReceivedGroup {
    pub(in crate::game) fn apply(self, world: &mut World, [x, y, z]: [i32; 3]) {
        world.set_connection_group(x, y, z, self.0, false);
    }
}

pub(in crate::game) type GroupUpdates = FxHashMap<[i32; 3], ReceivedGroup>;

pub(in crate::game) fn check_group_updates(
    world: &World,
    updates: impl IntoIterator<Item = ([i32; 3], ReceivedGroup)>,
    limit: usize,
) -> Result<()> {
    let (mut old, mut new) = (0, 0);
    for ([x, y, z], group) in updates {
        old += usize::from(world.connection_group(x, y, z).is_some());
        new += usize::from(group.0.is_some());
    }
    check_group_capacity(world, old, new, limit)
}

fn check_group_capacity(world: &World, old: usize, new: usize, limit: usize) -> Result<()> {
    if world.chunks.allows_connection_groups(old, new, limit) {
        Ok(())
    } else {
        Err(Error::new(format!(
            "connection groups: resident cell limit ({limit}) exceeded"
        )))
    }
}

/// Install the decoded map in the new column before the column, its remesh
/// diff or any of its load hooks can become visible.
pub(in crate::game) fn column_groups(
    chunk: &mut Chunk,
    world: &World,
    sections: &[Section],
    limit: usize,
) -> Result<()> {
    if sections.is_empty() {
        return Ok(());
    }
    let old = world
        .chunks
        .get(&(chunk.x, chunk.z))
        .map_or(0, |resident| resident.connection_groups.len());
    let new = sections.iter().map(|section| section.entries.len()).sum();
    check_group_capacity(world, old, new, limit)?;
    let mut seen = sunrise_level::fast_hash::FxHashSet::default();
    for section in sections {
        let [x, y, z] = section.pos;
        if (x, z) != (chunk.x, chunk.z) || !world.shape.has_section(y) || !seen.insert(y) {
            return Err(Error::new("connection groups: unexpected column section"));
        }
        for entry in &section.entries {
            let Some(group) = entry.group else {
                return Err(Error::new("connection groups: clear in full column"));
            };
            let [x, y, z] = entry.cell.at(section.pos);
            chunk.set_connection_group(x & 15, y, z & 15, Some(group));
        }
    }
    Ok(())
}

pub(in crate::game) fn block_groups(
    world: &World,
    packet: &C,
    sections: &[Section],
) -> Result<GroupUpdates> {
    let [section] = sections else {
        return Err(Error::new(
            "connection groups: block update needs one group section",
        ));
    };
    let mut updates = GroupUpdates::default();
    match packet {
        C::BlockUpdate(block) => {
            let [entry] = section.entries.as_slice() else {
                return Err(Error::new(
                    "connection groups: single block needs one group cell",
                ));
            };
            if entry.cell.at(section.pos) != block.pos {
                return Err(Error::new(
                    "connection groups: group cell not in block update",
                ));
            }
            updates.insert(block.pos, ReceivedGroup(entry.group));
        }
        C::SectionBlocksUpdate(blocks) if blocks.section == section.pos => {
            let mut positions = [0_u64; 4096 / 64];
            for &change in &blocks.changes {
                let index = (change & 0xFFF) as usize;
                positions[index / 64] |= 1 << (index % 64);
            }
            for entry in &section.entries {
                let index = entry.cell.index();
                if positions[index / 64] & (1 << (index % 64)) == 0 {
                    return Err(Error::new(
                        "connection groups: group cell not in section update",
                    ));
                }
                updates.insert(entry.cell.at(section.pos), ReceivedGroup(entry.group));
            }
        }
        _ => {
            return Err(Error::new(
                "connection groups: companion is not its block update",
            ));
        }
    }
    if updates
        .keys()
        .any(|&[x, y, z]| !world.is_loaded_at(x, y, z))
    {
        return Err(Error::new("connection groups: group cell is not loaded"));
    }
    Ok(updates)
}

#[cfg(feature = "client")]
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_level::chunk_map::ConnectionGroupCount;
    use sunrise_protocol::connection_groups::{Cell, Entry};
    use sunrise_world::registry::BlockStateId;

    #[test]
    fn full_credit_allows_a_resend_but_refuses_new_cells_before_adoption() {
        let mut world = World::new(BlockStateId(0), 0);
        world
            .chunks
            .track_connection_groups(ConnectionGroupCount::default());
        let mut original = Chunk::empty(0, 0);
        original.set_connection_group(0, 0, 0, Some(30));
        world.chunks.insert((0, 0), Arc::new(original));

        let section = |x| Section {
            pos: [x, 0, 0],
            entries: vec![Entry {
                cell: Cell::of([0, 0, 0]),
                group: Some(40),
            }],
        };
        let mut replacement = Chunk::empty(0, 0);
        column_groups(&mut replacement, &world, &[section(0)], 1).unwrap();
        assert_eq!(replacement.connection_group(0, 0, 0), Some(40));
        world.chunks.insert((0, 0), Arc::new(replacement));

        let mut new_column = Chunk::empty(1, 0);
        let error = column_groups(&mut new_column, &world, &[section(1)], 1).unwrap_err();
        assert_eq!(
            error.to_string(),
            "connection groups: resident cell limit (1) exceeded"
        );
        assert_eq!(new_column.connection_group(0, 0, 0), None);
        assert!(!world.chunks.contains_key(&(1, 0)));
    }
}
