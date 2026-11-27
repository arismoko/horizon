//! The game owns mod callback dispatch; the world owns the placement seam.
use mod_api::__private::RegisteredBlockEntityTransform;
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};
use sunrise_core::nbt::{Compound, JavaString, Tag};
use sunrise_world::{
    registry::BlockRegistry,
    worldgen::structure::{Mirror, Rotation},
};

struct Adapter(HashMap<JavaString, RegisteredBlockEntityTransform>);

impl sunrise_world::worldgen::structure::block_entity_transform::BlockEntityTransform for Adapter {
    fn transform(
        &self,
        position: [i32; 3],
        data: &mut Compound,
        rotation: Rotation,
        mirror: Mirror,
    ) {
        static IDENTITY: LazyLock<[JavaString; 4]> =
            LazyLock::new(|| ["id", "x", "y", "z"].map(JavaString::from));
        let Some(Tag::String(id)) = data.get(&IDENTITY[0]) else {
            return;
        };
        let Some((id, registration)) = self.0.get_key_value(id) else {
            return;
        };
        if !registration.owner.is_enabled() {
            return;
        }
        let transform = mod_api::StructureTransform::new(
            match rotation {
                Rotation::None => mod_api::StructureRotation::None,
                Rotation::Clockwise90 => mod_api::StructureRotation::Clockwise90,
                Rotation::Clockwise180 => mod_api::StructureRotation::Clockwise180,
                Rotation::Counterclockwise90 => mod_api::StructureRotation::Counterclockwise90,
            },
            match mirror {
                Mirror::None => mod_api::StructureMirror::None,
                Mirror::LeftRight => mod_api::StructureMirror::LeftRight,
                Mirror::FrontBack => mod_api::StructureMirror::FrontBack,
            },
        );
        // The transform edits payload fields, not the saved entity's identity. Keep just
        // those fields, not an undo copy of the working compound.
        let identity = IDENTITY.each_ref().map(|key| data.get(key).cloned());
        let result = registration.owner.call(
            "structure block entity transform",
            "block_entity_transform",
            || registration.step.transform(data, transform),
        );
        for (key, original) in IDENTITY.iter().zip(identity) {
            if let Some(original) = original {
                if let Some(current) = data.get_mut(key) {
                    *current = original;
                } else {
                    data.insert(key.clone(), original);
                }
            }
        }
        if let Ok(Err(error)) = result {
            log::warn!(
                "mod {}: block entity {} at {position:?} refused structure transform: {error}",
                registration.owner.id(),
                id.to_string_lossy(),
            );
        }
    }
}

/// Freeze the callbacks with the registry; unused placement performs one OnceLock lookup.
pub fn prepare(registry: &BlockRegistry) {
    if registry.structure_block_entity_transform.get().is_some() {
        return;
    }
    let steps: HashMap<_, _> = mod_api::__private::block_entity_transforms()
        .into_iter()
        .map(|(id, step)| (JavaString::from(id.as_str()), step))
        .collect();
    if !steps.is_empty() {
        let _ = registry
            .structure_block_entity_transform
            .set(Arc::new(Adapter(steps)));
    }
}
