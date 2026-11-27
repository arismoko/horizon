//! What the game tests do to the entities and chunks of a test's area:
//! `StructureUtils.clearSpaceForStructure`'s and `GameTestInfo.succeed`'s
//! entity removal, and the force-loaded chunks of
//! `TestInstanceBlockEntity.forceLoadChunks` and `GameTestRunner`'s release.

#[allow(unused_imports)]
pub(in crate::game) use sunrise_gameplay_common::test_entities::{Aabb, intersects, standing};
#[allow(unused_imports)]
pub use sunrise_gameplay_common::test_entities::{aabb_of, inflate};
