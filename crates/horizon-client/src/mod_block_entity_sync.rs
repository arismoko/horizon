//! Native block-entity types and their client-visible update tags.
use horizon_world::nbt::{Compound, Tag};
use std::sync::OnceLock;

pub type UpdateTag = fn(&str, &Compound, &mut Option<Compound>);

struct Registry {
    names: Vec<String>,
    update_tag: UpdateTag,
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();

/// Register the compiled native mods' block-entity types before connections start.
pub fn initialize(mut names: Vec<String>, update_tag: UpdateTag) -> Result<(), String> {
    if names.iter().any(|n| {
        n.split_once(':').is_none_or(|(namespace, path)| {
            namespace.is_empty()
                || namespace == "minecraft"
                || path.is_empty()
                || horizon_client_core::identifier::parse_identifier(n).as_deref() != Some(n)
        })
    }) {
        return Err("invalid native block-entity type".into());
    }
    names.sort();
    if names.windows(2).any(|w| w[0] == w[1]) {
        return Err("duplicate native block-entity type".into());
    }
    REGISTRY
        .set(Registry { names, update_tag })
        .map_err(|_| "native block-entity types already initialized".to_owned())
}

pub fn names() -> &'static [String] {
    REGISTRY.get().map_or(&[], |registry| &registry.names)
}

pub fn update_tag(id: &str, data: &Compound) -> Option<Tag> {
    let registry = REGISTRY.get()?;
    let mut tag = None;
    (registry.update_tag)(id, data, &mut tag);
    tag.map(Tag::Compound)
}

/// Apply an update only to a loaded mod block and the matching entity type.
pub fn apply_client_update(
    world: &mut crate::world::World,
    pos: [i32; 3],
    block: &str,
    id: &str,
    tag: &Tag,
) {
    use crate::containers::{BlockEntity, ModBlockEntity};
    if !world.loaded(pos[0], pos[2]) || block.starts_with("minecraft:") {
        return;
    }
    let key = (pos[0], pos[1], pos[2]);
    if world
        .block_entities
        .get(&key)
        .is_some_and(|entity| entity.id() != id)
    {
        return;
    }
    let Tag::Compound(fields) = tag else { return };
    world.block_entities.insert(
        key,
        BlockEntity::Mod(Box::new(ModBlockEntity {
            id: id.to_owned(),
            nbt: fields.clone(),
        })),
    );
}

/// A property-only state update keeps the existing client entity.
pub fn keep_client_entity(
    old_block: &str,
    new_block: &str,
    entity: Option<&crate::containers::BlockEntity>,
) -> bool {
    old_block == new_block && matches!(entity, Some(crate::containers::BlockEntity::Mod(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{chunk::Chunk, containers::BlockEntity, world::World};
    use horizon_world::registry::BlockStateId;
    use std::sync::Arc;

    #[test]
    fn client_updates_and_removal() {
        let mut world = World::new(BlockStateId(0), 0);
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let pos = [1, 5, 2];
        let mut data = Compound::new();
        data.insert("speed".into(), Tag::Int(3));
        apply_client_update(
            &mut world,
            pos,
            "example:block",
            "example:entity",
            &Tag::Compound(data.clone()),
        );
        let key = (1, 5, 2);
        let entity = world.block_entities.get(&key).unwrap();
        assert_eq!(entity.id(), "example:entity");
        assert!(keep_client_entity(
            "example:block",
            "example:block",
            Some(entity)
        ));
        assert!(!keep_client_entity(
            "example:block",
            "minecraft:air",
            Some(entity)
        ));
        data.insert("speed".into(), Tag::Int(4));
        apply_client_update(
            &mut world,
            pos,
            "example:block",
            "example:other",
            &Tag::Compound(data.clone()),
        );
        assert_eq!(
            world.block_entities.get(&key).unwrap().id(),
            "example:entity"
        );
        apply_client_update(
            &mut world,
            pos,
            "example:block",
            "example:entity",
            &Tag::Compound(data.clone()),
        );
        let Some(BlockEntity::Mod(entity)) = world.block_entities.get(&key) else {
            panic!("missing client entity")
        };
        assert_eq!(entity.nbt, data);
        // The block-change path removes the entity when the block type changes.
        world.block_entities.remove(&key);
        apply_client_update(
            &mut world,
            pos,
            "minecraft:air",
            "example:entity",
            &Tag::Compound(data.clone()),
        );
        assert!(world.block_entities.get(&key).is_none());
        world.chunks.remove(&(0, 0));
        apply_client_update(
            &mut world,
            pos,
            "example:block",
            "example:entity",
            &Tag::Compound(data),
        );
        assert!(world.block_entities.get(&key).is_none());
    }
}
