use sunrise_core::nbt::{Compound, Tag};
use sunrise_level::{
    block_entities::Pos,
    chunk::PendingContainerLoot,
    world::{BlockEntityExtras, World},
};

/// Mutable position-owned server data borrowed by the level adapter.
pub struct PositionState<'a> {
    pub gateways: &'a mut sunrise_level::fast_hash::FxHashMap<
        [i32; 3],
        sunrise_level::block_entity_ticks::EndGateway,
    >,
    pub hoppers:
        &'a mut std::collections::HashMap<Pos, sunrise_gameplay_common::hoppers::HopperState>,
}

pub enum PositionChange {
    Carry {
        from: Pos,
        to: Pos,
        partner: Option<Pos>,
    },
    Land {
        at: Pos,
    },
}

pub(in crate::game) fn take_position_data(world: &mut World, from: Pos) -> Compound {
    let mut extras = world.block_entity_extras(from);
    if extras.source.is_none() {
        extras.source = crate::game::spill_common::saved_entity(world, [from.0, from.1, from.2]);
    }
    let mut data = Compound::new();
    let name = world
        .block_names
        .remove(&from)
        .or_else(|| {
            extras
                .source
                .as_ref()
                .and_then(|t| t.get("CustomName"))
                .cloned()
        })
        .or_else(|| match world.block_entities.get(&from) {
            Some(sunrise_level::containers::BlockEntity::Mod(e)) => {
                e.nbt.get(&"CustomName".into()).cloned()
            }
            _ => None,
        });
    if let Some(name) = name {
        data.insert("Name".into(), name);
    }
    if let Some(source) = extras.source {
        data.insert("Source".into(), source);
    }
    if let Some(pending) = extras.pending_loot {
        let mut tag = Compound::new();
        pending.write_tags(&mut tag);
        data.insert("PendingLoot".into(), Tag::Compound(tag));
    }
    world.set_block_entity_extras(from, BlockEntityExtras::default());
    data
}
pub(in crate::game) fn land_position_data(world: &mut World, at: Pos, data: &Compound) {
    if let Some(name) = data.get(&"Name".into()) {
        world.block_names.insert(at, name.clone());
    }
    let pending_loot = data
        .get(&"PendingLoot".into())
        .and_then(PendingContainerLoot::from_tag);
    let mut source = data.get(&"Source".into()).cloned();
    if let Some(Tag::Compound(fields)) = &mut source {
        for (name, n) in [("x", at.0), ("y", at.1), ("z", at.2)] {
            fields.insert(name.into(), Tag::Int(n));
        }
    }
    world.set_block_entity_extras(
        at,
        BlockEntityExtras {
            source,
            pending_loot,
        },
    );
}
