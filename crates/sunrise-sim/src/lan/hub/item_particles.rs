//! ItemParticleOption's stack template uses the ordinary inventory component codec.
use super::*;
pub(super) fn encode(
    hub: &Hub,
    id: i32,
    options: &sunrise_level::particle_options::ParticleOptions,
) -> Option<Particle> {
    if options.ty != sunrise_level::particle_options::ParticleType::Item {
        return None;
    }
    let stack = options.item_stack.as_ref()?;
    encode_stack(hub, id, stack)
}

pub(super) fn encode_stack(
    hub: &Hub,
    id: i32,
    stack: &crate::items::ItemStack,
) -> Option<Particle> {
    let stack = crate::lan::guest::stack_to_network(
        |name| {
            hub.sync
                .builtin
                .get("minecraft:item")
                .and_then(|r| r.id(name))
                .unwrap_or(0)
        },
        |registry, name| hub.sync.id(registry, name),
        &Some(stack.clone()),
    )?;
    Some(Particle {
        id,
        data: Some(ParticleData::Item(stack)),
    })
}
