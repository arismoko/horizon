use std::io;
use sunrise_core::geometry::{Aabb, ShapeOp, Vec3, VoxelShape};
use sunrise_world::registry::BlockStateId;

pub fn ray_shape_from_boxes(boxes: &[[f64; 6]]) -> io::Result<VoxelShape> {
    let mut shape = VoxelShape::empty();
    for b in boxes {
        let part = VoxelShape::from_box(Aabb::new(
            Vec3::new(b[0], b[1], b[2]),
            Vec3::new(b[3], b[4], b[5]),
        ))
        .map_err(|e| io::Error::other(format!("{e:?}")))?;
        shape = shape
            .join(&part, ShapeOp::OR)
            .map_err(|e| io::Error::other(format!("{e:?}")))?;
    }
    Ok(shape)
}

pub(in crate::game) fn ray_interaction_shape(
    registry: &sunrise_world::registry::BlockRegistry,
    state: BlockStateId,
) -> io::Result<VoxelShape> {
    let state = registry
        .state(state)
        .ok_or_else(|| io::Error::other("unknown block state"))?;
    ray_shape_from_boxes(&super::interaction_shapes::boxes(
        &state.name,
        state.properties.get("facing").map(String::as_str),
    ))
}
