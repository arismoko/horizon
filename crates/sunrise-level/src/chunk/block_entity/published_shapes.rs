//! The collision and outline shapes a mod block entity publishes for its block
//! (`BlockEntityRef::set_shapes`): checked once when published, built into shapes once, and
//! then only read by position.
use mod_api::{BlockEntityShapes, BlockEntityShapesError};
use sunrise_core::{
    geometry::{Aabb, ShapeOp, Vec3, VoxelShape},
    nbt::{Compound, Tag},
};

/// The most boxes in one published list: a bound on what a block entity makes every client
/// receive and keep (vanilla has no such limit; a declared shape table has no box limit).
pub const MAX_BOXES: usize = 64;
/// Where a block entity's published shapes sit in its saved compound and its update tag, the
/// name no mod's own fields can use (`ModBlockEntity::merge_update_tag` takes it out).
pub const SHAPES_KEY: &str = "sunrise:published_shapes";

/// Published shapes, validated: the boxes as given, for saving and sending, and the shapes
/// they build, for the queries.
#[derive(Debug)]
pub struct PublishedShapes {
    boxes: BlockEntityShapes,
    outline: VoxelShape,
    collision: VoxelShape,
}

/// A collision shape built from boxes that passed the bounds of a published shape: only
/// [`CheckedShape::new`] makes one, so whoever holds it needs no second check.
#[derive(Clone, Debug)]
pub struct CheckedShape(VoxelShape);
impl CheckedShape {
    /// Checks `boxes` against the bounds (at most 64, finite, minimum at most maximum) and
    /// builds the shape they make.
    pub fn new(boxes: &[[f64; 6]]) -> Result<Self, BlockEntityShapesError> {
        PublishedShapes::build(boxes, true).map(Self)
    }
    pub fn into_shape(self) -> VoxelShape {
        self.0
    }
}

impl PublishedShapes {
    /// Checks `shapes` against the limits of `BlockEntityShapes` and builds them. A box under
    /// 1e-7 thick on an axis builds an empty shape, as vanilla's `Shapes.create` does.
    pub fn new(shapes: BlockEntityShapes) -> Result<Self, BlockEntityShapesError> {
        let outline = Self::build(&shapes.outline, false)?;
        let collision = Self::build(&shapes.collision, true)?;
        Ok(Self {
            boxes: shapes,
            outline,
            collision,
        })
    }

    /// Checks `boxes` against the limits of `BlockEntityShapes`: at most `MAX_BOXES`, every
    /// coordinate finite and each minimum at most its maximum. `collision` names the list in
    /// the error.
    pub fn check_boxes(boxes: &[[f64; 6]], collision: bool) -> Result<(), BlockEntityShapesError> {
        if boxes.len() > MAX_BOXES {
            return Err(BlockEntityShapesError::TooManyBoxes {
                collision,
                count: boxes.len() as u64,
                limit: MAX_BOXES as u64,
            });
        }
        for (index, bounds) in boxes.iter().enumerate() {
            let index = index as u64;
            if bounds.iter().any(|value| !value.is_finite()) {
                return Err(BlockEntityShapesError::NotFinite { collision, index });
            }
            if (0..3).any(|axis| bounds[axis] > bounds[axis + 3]) {
                return Err(BlockEntityShapesError::InvertedBox { collision, index });
            }
        }
        Ok(())
    }

    /// The shape `boxes` build once they pass [`Self::check_boxes`].
    fn build(boxes: &[[f64; 6]], collision: bool) -> Result<VoxelShape, BlockEntityShapesError> {
        Self::check_boxes(boxes, collision)?;
        let mut shape = VoxelShape::empty();
        for bounds in boxes {
            let part = VoxelShape::from_box(Aabb::new(
                Vec3::new(bounds[0], bounds[1], bounds[2]),
                Vec3::new(bounds[3], bounds[4], bounds[5]),
            ))
            .map_err(|_| BlockEntityShapesError::Unbuildable)?;
            shape = shape
                .join_unoptimized(&part, ShapeOp::OR)
                .map_err(|_| BlockEntityShapesError::Unbuildable)?;
        }
        Ok(shape)
    }

    /// Whether a box lies beyond one block of the block, where collision never finds it.
    pub fn reaches_past_one_cell(&self) -> bool {
        self.boxes
            .outline
            .iter()
            .chain(&self.boxes.collision)
            .flatten()
            .any(|value| !(-1.0..=2.0).contains(value))
    }

    /// What the crosshair picks.
    pub fn outline(&self) -> &VoxelShape {
        &self.outline
    }

    /// What entities collide with.
    pub fn collision(&self) -> &VoxelShape {
        &self.collision
    }

    /// The boxes as published.
    pub fn boxes(&self) -> &BlockEntityShapes {
        &self.boxes
    }

    /// The saved and sent form: each list's boxes as the bits of their coordinates, six to
    /// a box, so no coordinate changes on the way.
    pub fn to_tag(&self) -> Tag {
        let bits = |boxes: &[[f64; 6]]| {
            Tag::LongArray(
                boxes
                    .iter()
                    .flatten()
                    .map(|value| value.to_bits() as i64)
                    .collect(),
            )
        };
        Tag::Compound(Compound::from([
            ("outline".into(), bits(&self.boxes.outline)),
            ("collision".into(), bits(&self.boxes.collision)),
        ]))
    }

    /// What [`Self::to_tag`] wrote, checked as a published shape is. `None` for a tag that is
    /// malformed or over a limit: saved data and a server's tags are not trusted.
    pub fn from_tag(tag: &Tag) -> Option<Self> {
        let boxes = |name: &str| -> Option<Vec<[f64; 6]>> {
            let Some(Tag::LongArray(bits)) = tag.get(name) else {
                return None;
            };
            if bits.len() % 6 != 0 || bits.len() / 6 > MAX_BOXES {
                return None;
            }
            Some(
                bits.as_chunks::<6>()
                    .0
                    .iter()
                    .map(|chunk| chunk.map(|bits| f64::from_bits(bits as u64)))
                    .collect(),
            )
        };
        Self::new(BlockEntityShapes {
            outline: boxes("outline")?,
            collision: boxes("collision")?,
        })
        .ok()
    }
}

impl PartialEq for PublishedShapes {
    fn eq(&self, other: &Self) -> bool {
        self.boxes == other.boxes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shapes(outline: Vec<[f64; 6]>, collision: Vec<[f64; 6]>) -> BlockEntityShapes {
        BlockEntityShapes { outline, collision }
    }

    #[test]
    fn a_shape_over_a_limit_is_refused_by_name_and_never_clamped() {
        let unit = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let refused = |outline: Vec<[f64; 6]>, collision: Vec<[f64; 6]>| {
            PublishedShapes::new(shapes(outline, collision)).err()
        };
        assert_eq!(
            refused(vec![unit; 65], vec![]),
            Some(BlockEntityShapesError::TooManyBoxes {
                collision: false,
                count: 65,
                limit: 64
            })
        );
        assert_eq!(
            refused(vec![], vec![[0.0, 0.0, 0.0, 1.0, f64::NAN, 1.0]]),
            Some(BlockEntityShapesError::NotFinite {
                collision: true,
                index: 0
            })
        );
        assert_eq!(
            refused(vec![unit, [0.5, 0.0, 0.0, 0.25, 1.0, 1.0]], vec![]),
            Some(BlockEntityShapesError::InvertedBox {
                collision: false,
                index: 1
            })
        );
        // The cap itself, a box far past the block (stored as given), and a box
        // thinner than 1e-7 (an empty one) are accepted.
        let far = [10.0, 0.0, 0.0, 12.0, 1.0, 1.0];
        let edge = PublishedShapes::new(shapes(
            vec![unit; 64],
            vec![far, [0.0, 0.0, 0.0, 1.0, 5e-8, 1.0]],
        ))
        .unwrap();
        assert!(edge.reaches_past_one_cell());
        assert_eq!(edge.boxes().collision[0], far);
        assert!(
            !PublishedShapes::new(shapes(vec![unit], vec![]))
                .unwrap()
                .reaches_past_one_cell()
        );
        let thin =
            PublishedShapes::new(shapes(vec![], vec![[0.0, 0.0, 0.0, 1.0, 5e-8, 1.0]])).unwrap();
        assert!(thin.collision().is_empty());
    }

    #[test]
    fn published_shapes_round_trip_through_their_tag_and_bad_tags_are_ignored() {
        let published = PublishedShapes::new(shapes(
            vec![[0.0, 0.0, 0.0, 1.0, 0.1, 1.0]],
            vec![
                [0.25, 0.0, 0.25, 0.75, 1.5, 0.75],
                [-0.25, 0.0, 0.0, 0.0, 1.0, 1.0],
            ],
        ))
        .unwrap();
        let back = PublishedShapes::from_tag(&published.to_tag()).unwrap();
        assert_eq!(back, published);
        assert_eq!(back.collision().boxes().len(), 2);
        let ragged = Tag::Compound(Compound::from([
            ("outline".into(), Tag::LongArray(vec![0; 5])),
            ("collision".into(), Tag::LongArray(vec![])),
        ]));
        assert!(PublishedShapes::from_tag(&ragged).is_none());
        let over = Tag::Compound(Compound::from([
            ("outline".into(), Tag::LongArray(vec![0; 6 * 65])),
            ("collision".into(), Tag::LongArray(vec![])),
        ]));
        assert!(PublishedShapes::from_tag(&over).is_none());
    }
}
