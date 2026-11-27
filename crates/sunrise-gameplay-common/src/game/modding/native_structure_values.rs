use mod_api::{StructureMirror, StructureRotation};
use sunrise_level::structure_block::parse_identifier;
use sunrise_world::worldgen::structure::{Mirror, Rotation};

/// The engine's rotation for an API one; `None` for one this build doesn't know.
pub fn engine_rotation(rotation: StructureRotation) -> Option<Rotation> {
    Some(match rotation {
        StructureRotation::None => Rotation::None,
        StructureRotation::Clockwise90 => Rotation::Clockwise90,
        StructureRotation::Clockwise180 => Rotation::Clockwise180,
        StructureRotation::Counterclockwise90 => Rotation::Counterclockwise90,
        _ => return None,
    })
}

/// The engine's mirror for an API one; `None` for one this build doesn't know.
pub fn engine_mirror(mirror: StructureMirror) -> Option<Mirror> {
    Some(match mirror {
        StructureMirror::None => Mirror::None,
        StructureMirror::LeftRight => Mirror::LeftRight,
        StructureMirror::FrontBack => Mirror::FrontBack,
        _ => return None,
    })
}

/// `Identifier.tryParse`, less the paths the world's `generated` folder
/// refuses (`FileUtil.isPathNormalized`): one with a `.` or `..` part.
pub fn template_id(id: &str) -> Option<String> {
    let id = parse_identifier(id)?;
    let (_, path) = id.split_once(':')?;
    path.split('/')
        .all(|part| part != "." && part != "..")
        .then_some(id)
}
